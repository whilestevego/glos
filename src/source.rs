//! Turns two files, two folders or a git repo into one list of changes.
//!
//! Every source ends up as the same `Change`s: an old side and a new side per path. Contents are
//! read on demand (`Source::read`), so the list is ready before any file is loaded or diffed.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use gix::bstr::ByteSlice;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    File,
    Executable,
    Symlink,
}

/// Where one side's content lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// The side doesn't exist: the file was added (old) or deleted (new).
    None,
    /// A file on disk; for a symlink, its target is the content.
    File(PathBuf),
    /// A git blob; for a symlink, its target.
    Blob(gix::ObjectId),
    /// Content kept by glos: a file as it was when you last submitted.
    Bytes(std::sync::Arc<[u8]>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Added,
    Deleted,
    Modified,
    Renamed,
    /// Only the mode changed (e.g. made executable).
    ModeChanged,
}

#[derive(Debug, Clone)]
pub struct Change {
    /// Relative, `/`-separated: the new path, or the old one for a deletion.
    pub path: String,
    /// The old path, when the old side exists.
    pub old_path: Option<String>,
    pub old: Content,
    pub new: Content,
    pub old_mode: Option<Mode>,
    pub new_mode: Option<Mode>,
    /// The content didn't change (a pure rename or mode change).
    pub same_content: bool,
}

impl Change {
    pub fn kind(&self) -> Kind {
        match (&self.old, &self.new) {
            (Content::None, _) => Kind::Added,
            (_, Content::None) => Kind::Deleted,
            _ if self.old_path.as_deref() != Some(&self.path) => Kind::Renamed,
            _ if self.same_content => Kind::ModeChanged,
            _ => Kind::Modified,
        }
    }

    /// Lockfiles and minified files start collapsed. (`linguist-generated` comes from
    /// `.gitattributes` and is checked by the git source.)
    pub fn looks_generated(&self) -> bool {
        const LOCKFILES: &[&str] = &[
            "Cargo.lock",
            "package-lock.json",
            "npm-shrinkwrap.json",
            "yarn.lock",
            "pnpm-lock.yaml",
            "bun.lockb",
            "bun.lock",
            "Gemfile.lock",
            "poetry.lock",
            "uv.lock",
            "Pipfile.lock",
            "composer.lock",
            "go.sum",
            "flake.lock",
            "mix.lock",
            "pubspec.lock",
            "Package.resolved",
            "Podfile.lock",
        ];
        let name = self.path.rsplit('/').next().unwrap_or(&self.path);
        LOCKFILES.contains(&name)
            || [".min.js", ".min.css", ".min.mjs", ".map"]
                .iter()
                .any(|s| name.ends_with(s))
    }
}

/// Git treats content with a NUL in its first 8000 bytes as binary; so does glos.
pub fn is_binary(content: &[u8]) -> bool {
    content[..content.len().min(8000)].contains(&0)
}

pub struct Git {
    pub repo: gix::ThreadSafeRepository,
    /// The shared git dir (the same for every worktree of a repo), canonical.
    pub common_dir: PathBuf,
    pub worktree: PathBuf,
    /// `None` in detached HEAD.
    pub branch: Option<String>,
    /// The base ref's name, e.g. `main` or `origin/main`.
    pub base: String,
    pub merge_base: gix::ObjectId,
    pub head: gix::ObjectId,
}

pub struct Source {
    pub changes: Vec<Change>,
    /// The root the new side's relative paths are under.
    pub root: PathBuf,
    pub git: Option<Git>,
    /// The two paths given to `glos diff`, canonical; `None` for git.
    pub paths: Option<(PathBuf, PathBuf)>,
    /// Paths marked `linguist-generated` in `.gitattributes`.
    generated: Vec<String>,
}

impl Source {
    /// The content of one side, or `None` if that side doesn't exist.
    pub fn read(&self, content: &Content) -> Result<Option<Vec<u8>>> {
        Ok(match content {
            Content::None => None,
            Content::File(path) => {
                let meta = std::fs::symlink_metadata(path)
                    .with_context(|| format!("reading {}", path.display()))?;
                if meta.is_symlink() {
                    Some(
                        std::fs::read_link(path)?
                            .into_os_string()
                            .into_encoded_bytes(),
                    )
                } else {
                    Some(
                        std::fs::read(path)
                            .with_context(|| format!("reading {}", path.display()))?,
                    )
                }
            }
            Content::Bytes(b) => Some(b.to_vec()),
            Content::Blob(id) => {
                let git = self
                    .git
                    .as_ref()
                    .ok_or_else(|| anyhow!("blob without a git source"))?;
                Some(git.repo.to_thread_local().find_object(*id)?.detach().data)
            }
        })
    }

    /// Starts collapsed: a lockfile, a minified file, or `linguist-generated`.
    pub fn collapsed(&self, change: &Change) -> bool {
        change.looks_generated() || self.generated.contains(&change.path)
    }

    /// `glos diff old new`: two files or two folders, like `git diff --no-index`.
    pub fn diff(old: &Path, new: &Path) -> Result<Source> {
        let (old_meta, new_meta) = (std::fs::metadata(old), std::fs::metadata(new));
        let (old_meta, new_meta) = (
            old_meta.with_context(|| format!("{}", old.display()))?,
            new_meta.with_context(|| format!("{}", new.display()))?,
        );
        match (old_meta.is_dir(), new_meta.is_dir()) {
            (false, false) => Self::files(old, new),
            (true, true) => Self::folders(old, new),
            _ => bail!("compare two files or two folders, not one of each"),
        }
    }

    fn files(old: &Path, new: &Path) -> Result<Source> {
        let name = new
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let old_bytes = std::fs::read(old)?;
        let new_bytes = std::fs::read(new)?;
        let change = Change {
            path: name.clone(),
            old_path: Some(name),
            old: Content::File(old.to_owned()),
            new: Content::File(new.to_owned()),
            old_mode: Some(disk_mode(old)?),
            new_mode: Some(disk_mode(new)?),
            same_content: old_bytes == new_bytes,
        };
        let unchanged = change.same_content && change.old_mode == change.new_mode;
        Ok(Source {
            changes: if unchanged { vec![] } else { vec![change] },
            root: new.parent().unwrap_or(Path::new(".")).to_owned(),
            git: None,
            paths: Some((old.canonicalize()?, new.canonicalize()?)),
            generated: vec![],
        })
    }

    fn folders(old: &Path, new: &Path) -> Result<Source> {
        let (old_files, new_files) = (walk(old)?, walk(new)?);
        let mut changes = vec![];
        let mut added = vec![];
        let mut deleted = vec![];
        for (path, new_mode) in &new_files {
            let Some(old_mode) = old_files.get(path) else {
                added.push(path.clone());
                continue;
            };
            let same = read_side(&old.join(path))? == read_side(&new.join(path))?;
            if !same || old_mode != new_mode {
                changes.push(Change {
                    path: path.clone(),
                    old_path: Some(path.clone()),
                    old: Content::File(old.join(path)),
                    new: Content::File(new.join(path)),
                    old_mode: Some(*old_mode),
                    new_mode: Some(*new_mode),
                    same_content: same,
                });
            }
        }
        for path in old_files.keys().filter(|p| !new_files.contains_key(*p)) {
            deleted.push(path.clone());
        }

        // Renames: a deleted and an added file with identical content.
        let mut by_content: HashMap<Vec<u8>, Vec<String>> = HashMap::new();
        for path in &deleted {
            by_content
                .entry(read_side(&old.join(path))?)
                .or_default()
                .push(path.clone());
        }
        for path in added {
            let content = read_side(&new.join(&path))?;
            let from = by_content.get_mut(&content).and_then(|v| v.pop());
            if let Some(from) = &from {
                deleted.retain(|d| d != from);
            }
            changes.push(Change {
                old: from
                    .as_ref()
                    .map_or(Content::None, |f| Content::File(old.join(f))),
                old_mode: from.as_ref().map(|f| old_files[f]),
                same_content: from.is_some(),
                old_path: from,
                new: Content::File(new.join(&path)),
                new_mode: Some(new_files[&path]),
                path,
            });
        }
        for path in deleted {
            changes.push(Change {
                old: Content::File(old.join(&path)),
                old_mode: Some(old_files[&path]),
                old_path: Some(path.clone()),
                new: Content::None,
                new_mode: None,
                same_content: false,
                path,
            });
        }
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Source {
            changes,
            root: new.to_owned(),
            git: None,
            paths: Some((old.canonicalize()?, new.canonicalize()?)),
            generated: vec![],
        })
    }

    /// `glos` in a repo: the branch since it left the base, uncommitted and untracked files
    /// included. `base` overrides the default (origin's default branch, else main or master).
    pub fn git(path: &Path, base: Option<&str>) -> Result<Source> {
        let repo = gix::discover(path).context("not in a git repository")?;
        let worktree = repo
            .workdir()
            .context("a bare repository has no worktree")?
            .canonicalize()?;
        let head = repo
            .head_commit()
            .context("the repository has no commits")?
            .id;
        let branch = repo.head_name()?.map(|n| n.shorten().to_string());
        let (base, base_id) = find_base(&repo, base)?;
        let merge_base = repo
            .merge_base(head, base_id)?
            .with_context(|| format!("this branch shares no history with {base}"))?
            .detach();
        let base_tree = repo.find_commit(merge_base)?.tree()?;

        // One status pass: merge-base tree → index (committed and staged), then index →
        // worktree (unstaged, deleted, untracked). Keyed by new path → old path.
        let mut paths: BTreeMap<String, Option<String>> = BTreeMap::new();
        let status = repo
            .status(gix::progress::Discard)?
            .head_tree(base_tree.id)
            .untracked_files(gix::status::UntrackedFiles::Files)
            .index_worktree_rewrites(Some(Default::default()))
            .tree_index_track_renames(gix::status::tree_index::TrackRenames::AsConfigured);
        use gix::status::{Item, index_worktree};
        for item in status.into_iter(Vec::<gix::bstr::BString>::new())? {
            match item? {
                Item::TreeIndex(gix::diff::index::Change::Rewrite {
                    source_location,
                    location,
                    ..
                }) => {
                    paths.insert(
                        location.to_str_lossy().into(),
                        Some(source_location.to_str_lossy().into()),
                    );
                }
                Item::TreeIndex(change) => {
                    let p: String = change.location().to_str_lossy().into();
                    paths.entry(p.clone()).or_insert(Some(p));
                }
                Item::IndexWorktree(index_worktree::Item::Rewrite {
                    source,
                    dirwalk_entry,
                    ..
                }) => {
                    // Moved on disk without `git mv`: carry the old side over.
                    let src: String = source.rela_path().to_str_lossy().into();
                    let old = paths.remove(&src).flatten().unwrap_or(src);
                    paths.insert(dirwalk_entry.rela_path.to_str_lossy().into(), Some(old));
                }
                Item::IndexWorktree(index_worktree::Item::DirectoryContents { entry, .. }) => {
                    if entry.status == gix::dir::entry::Status::Untracked {
                        paths
                            .entry(entry.rela_path.to_str_lossy().into())
                            .or_insert(None);
                    }
                }
                Item::IndexWorktree(index_worktree::Item::Modification { rela_path, .. }) => {
                    let p: String = rela_path.to_str_lossy().into();
                    paths.entry(p.clone()).or_insert(Some(p));
                }
            }
        }

        let mut changes = vec![];
        for (path, old_path) in paths {
            // The old side only exists if the path is in the merge-base tree.
            let old_entry = match &old_path {
                Some(p) => base_tree.lookup_entry_by_path(p)?,
                None => None,
            };
            let (old, old_mode) = match &old_entry {
                Some(e) => (Content::Blob(e.object_id()), Some(tree_mode(e.mode()))),
                None => (Content::None, None),
            };
            let on_disk = worktree.join(&path);
            let (new, new_mode) = match std::fs::symlink_metadata(&on_disk) {
                Ok(_) => (Content::File(on_disk.clone()), Some(disk_mode(&on_disk)?)),
                Err(_) => (Content::None, None),
            };
            if old == Content::None && new == Content::None {
                continue; // added and then deleted again
            }
            let same_content = match (&old_entry, &new) {
                (Some(e), Content::File(f)) => blob_id(&repo, f)? == Some(e.object_id()),
                _ => false,
            };
            let old_path = old_entry.is_some().then(|| old_path.unwrap());
            if same_content && old_path.as_deref() == Some(&path) && old_mode == new_mode {
                continue; // changed and then changed back
            }
            let path = if new == Content::None {
                old_path.clone().unwrap()
            } else {
                path
            };
            changes.push(Change {
                path,
                old_path,
                old,
                new,
                old_mode,
                new_mode,
                same_content,
            });
        }

        drop(base_tree);
        let generated = linguist_generated(&repo, &changes)?;
        Ok(Source {
            changes,
            root: worktree.clone(),
            generated,
            paths: None,
            git: Some(Git {
                common_dir: repo.common_dir().canonicalize()?,
                worktree,
                branch,
                base,
                merge_base,
                head,
                repo: repo.into_sync(),
            }),
        })
    }
}

/// A commit of the branch, for the per-commit scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub id: gix::ObjectId,
    /// The first line of its message.
    pub summary: String,
}

impl Source {
    /// The branch's commits since its base, oldest first, following first parents.
    pub fn commits(&self) -> Result<Vec<Commit>> {
        let g = self.git.as_ref().context("commits need a git repository")?;
        let repo = g.repo.to_thread_local();
        let mut out = vec![];
        for info in repo
            .rev_walk([g.head])
            .with_hidden([g.merge_base])
            .first_parent_only()
            .all()?
        {
            let id = info?.id;
            let commit = repo.find_commit(id)?;
            let summary = commit.message()?.summary().to_string();
            out.push(Commit { id, summary });
        }
        out.reverse();
        Ok(out)
    }

    /// One commit's changes: its first parent's tree against its own.
    pub fn commit(&self, id: gix::ObjectId) -> Result<Source> {
        let g = self.git.as_ref().context("commits need a git repository")?;
        let repo = g.repo.to_thread_local();
        let commit = repo.find_commit(id)?;
        let tree = commit.tree()?;
        let parent = match commit.parent_ids().next() {
            Some(p) => Some(repo.find_commit(p)?.tree()?),
            None => None,
        };
        use gix::object::tree::diff::ChangeDetached as C;
        let mut changes = vec![];
        for c in repo.diff_tree_to_tree(parent.as_ref(), Some(&tree), None)? {
            let blob = |m: gix::object::tree::EntryMode| m.is_blob() || m.is_link();
            let change = match c {
                C::Addition {
                    location,
                    entry_mode,
                    id,
                    ..
                } if blob(entry_mode) => Change {
                    path: location.to_string(),
                    old_path: None,
                    old: Content::None,
                    new: Content::Blob(id),
                    old_mode: None,
                    new_mode: Some(tree_mode(entry_mode)),
                    same_content: false,
                },
                C::Deletion {
                    location,
                    entry_mode,
                    id,
                    ..
                } if blob(entry_mode) => Change {
                    path: location.to_string(),
                    old_path: Some(location.to_string()),
                    old: Content::Blob(id),
                    new: Content::None,
                    old_mode: Some(tree_mode(entry_mode)),
                    new_mode: None,
                    same_content: false,
                },
                C::Modification {
                    location,
                    previous_entry_mode,
                    previous_id,
                    entry_mode,
                    id,
                } if blob(entry_mode) => Change {
                    path: location.to_string(),
                    old_path: Some(location.to_string()),
                    old: Content::Blob(previous_id),
                    new: Content::Blob(id),
                    old_mode: Some(tree_mode(previous_entry_mode)),
                    new_mode: Some(tree_mode(entry_mode)),
                    same_content: previous_id == id,
                },
                C::Rewrite {
                    source_location,
                    source_entry_mode,
                    source_id,
                    location,
                    entry_mode,
                    id,
                    ..
                } if blob(entry_mode) => Change {
                    path: location.to_string(),
                    old_path: Some(source_location.to_string()),
                    old: Content::Blob(source_id),
                    new: Content::Blob(id),
                    old_mode: Some(tree_mode(source_entry_mode)),
                    new_mode: Some(tree_mode(entry_mode)),
                    same_content: source_id == id,
                },
                _ => continue,
            };
            changes.push(change);
        }
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        let generated = linguist_generated(&repo, &changes)?;
        Ok(Source {
            changes,
            root: self.root.clone(),
            git: Some(Git {
                repo: g.repo.clone(),
                common_dir: g.common_dir.clone(),
                worktree: g.worktree.clone(),
                branch: g.branch.clone(),
                base: g.base.clone(),
                merge_base: g.merge_base,
                head: g.head,
            }),
            paths: self.paths.clone(),
            generated,
        })
    }

    /// The changes since you last submitted: each file now against its content in `snapshot`
    /// (path to content). Files reverted since count too; files unchanged since don't.
    pub fn since(mut self, snapshot: &HashMap<String, Vec<u8>>) -> Result<Source> {
        let mut changes = vec![];
        for c in &self.changes {
            let now = self.read(&c.new)?;
            let then = snapshot.get(&c.path);
            if now.as_ref() == then {
                continue;
            }
            changes.push(Change {
                path: c.path.clone(),
                old_path: then.map(|_| c.path.clone()),
                old: then.map_or(Content::None, |b| Content::Bytes(b.as_slice().into())),
                new: c.new.clone(),
                old_mode: then.and(c.new_mode),
                new_mode: c.new_mode,
                same_content: false,
            });
        }
        // In the snapshot but not changed any more: reverted to the base since.
        for (path, then) in snapshot {
            if self.changes.iter().any(|c| &c.path == path) {
                continue;
            }
            let disk = self.root.join(path);
            let now = std::fs::read(&disk).ok();
            if now.as_ref() == Some(then) {
                continue;
            }
            changes.push(Change {
                path: path.clone(),
                old_path: Some(path.clone()),
                old: Content::Bytes(then.as_slice().into()),
                new: if now.is_some() {
                    Content::File(disk.clone())
                } else {
                    Content::None
                },
                old_mode: Some(Mode::File),
                new_mode: now.is_some().then(|| disk_mode(&disk)).transpose()?,
                same_content: false,
            });
        }
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        self.changes = changes;
        Ok(self)
    }
}

fn find_base(repo: &gix::Repository, given: Option<&str>) -> Result<(String, gix::ObjectId)> {
    let mut candidates: Vec<String> = vec![];
    match given {
        Some(r) => candidates.push(r.into()),
        None => {
            // origin/HEAD points at origin's default branch.
            if let Ok(Some(r)) = repo.try_find_reference("refs/remotes/origin/HEAD")
                && let Some(name) = r.target().try_name()
            {
                candidates.push(name.shorten().to_string());
            }
            candidates.extend(["main", "master", "origin/main", "origin/master"].map(String::from));
        }
    }
    for c in &candidates {
        if let Ok(id) = repo.rev_parse_single(c.as_str()) {
            return Ok((c.clone(), id.detach()));
        }
    }
    bail!("no base branch found (tried {})", candidates.join(", "))
}

/// The id git would give `path`'s content, to tell an unchanged file from a changed one cheaply.
fn blob_id(repo: &gix::Repository, path: &Path) -> Result<Option<gix::ObjectId>> {
    let content = read_side(path)?;
    Ok(Some(gix::objs::compute_hash(
        repo.object_hash(),
        gix::object::Kind::Blob,
        &content,
    )?))
}

fn linguist_generated(repo: &gix::Repository, changes: &[Change]) -> Result<Vec<String>> {
    let index = repo.index_or_empty()?;
    let mut attrs = repo.attributes_only(
        &index,
        gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
    )?;
    let mut out = attrs.selected_attribute_matches(["linguist-generated"]);
    let mut generated = vec![];
    for c in changes {
        let platform = attrs.at_entry(c.path.as_str(), None)?;
        if platform.matching_attributes(&mut out) {
            let set = out.iter_selected().next().is_some_and(|m| {
                matches!(m.assignment.state, gix::attrs::StateRef::Set)
                    || matches!(m.assignment.state, gix::attrs::StateRef::Value(v) if v.as_bstr() == "true")
            });
            if set {
                generated.push(c.path.clone());
            }
        }
    }
    Ok(generated)
}

fn tree_mode(mode: gix::object::tree::EntryMode) -> Mode {
    if mode.is_link() {
        Mode::Symlink
    } else if mode.is_executable() {
        Mode::Executable
    } else {
        Mode::File
    }
}

fn disk_mode(path: &Path) -> Result<Mode> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::symlink_metadata(path)?;
    Ok(if meta.is_symlink() {
        Mode::Symlink
    } else if meta.permissions().mode() & 0o111 != 0 {
        Mode::Executable
    } else {
        Mode::File
    })
}

/// A side's bytes: the file's content, or a symlink's target.
fn read_side(path: &Path) -> Result<Vec<u8>> {
    if std::fs::symlink_metadata(path)?.is_symlink() {
        Ok(std::fs::read_link(path)?
            .into_os_string()
            .into_encoded_bytes())
    } else {
        Ok(std::fs::read(path)?)
    }
}

/// Every file and symlink under `root`, relative and `/`-separated, honouring `.gitignore`
/// even outside a git repo. Dotfiles are included; `.git` is not.
fn walk(root: &Path) -> Result<BTreeMap<String, Mode>> {
    let mut files = BTreeMap::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .require_git(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    for entry in walker {
        let entry = entry?;
        let Some(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        files.insert(rel, disk_mode(entry.path())?);
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, process::Command};

    fn summary(s: &Source) -> Vec<String> {
        s.changes
            .iter()
            .map(|c| {
                let from = match (&c.old_path, c.kind()) {
                    (Some(o), Kind::Renamed) => format!(" <- {o}"),
                    _ => String::new(),
                };
                format!("{:?} {}{from}", c.kind(), c.path)
            })
            .collect()
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn chmod_x(path: &Path) {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn two_files() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a.rs"), d.path().join("b.rs"));
        write(&a, "one\n");
        write(&b, "two\n");
        let s = Source::diff(&a, &b).unwrap();
        assert_eq!(summary(&s), ["Modified b.rs"]);
        assert_eq!(s.read(&s.changes[0].old).unwrap().unwrap(), b"one\n");

        write(&b, "one\n");
        assert!(Source::diff(&a, &b).unwrap().changes.is_empty());
        assert!(Source::diff(&a, d.path()).is_err());
    }

    #[test]
    fn two_folders() {
        let d = tempfile::tempdir().unwrap();
        let (old, new) = (d.path().join("old"), d.path().join("new"));
        for root in [&old, &new] {
            write(&root.join("same.txt"), "same\n");
            write(&root.join("src/edit.rs"), "fn a() {}\n");
            write(&root.join("run.sh"), "echo\n");
        }
        write(&new.join("src/edit.rs"), "fn b() {}\n");
        chmod_x(&new.join("run.sh"));
        write(&old.join("gone.txt"), "bye\n");
        write(&new.join("fresh.txt"), "hi\n");
        write(&old.join("before.txt"), "moved content\n");
        write(&new.join("sub/after.txt"), "moved content\n");
        std::os::unix::fs::symlink("same.txt", new.join("link")).unwrap();
        write(&new.join(".gitignore"), "*.log\n");
        write(&new.join("debug.log"), "noise\n");
        write(&new.join(".git/HEAD"), "not a repo\n");

        let s = Source::diff(&old, &new).unwrap();
        assert_eq!(
            summary(&s),
            [
                "Added .gitignore",
                "Added fresh.txt",
                "Deleted gone.txt",
                "Added link",
                "ModeChanged run.sh",
                "Modified src/edit.rs",
                "Renamed sub/after.txt <- before.txt",
            ]
        );
        let link = s.changes.iter().find(|c| c.path == "link").unwrap();
        assert_eq!(link.new_mode, Some(Mode::Symlink));
        assert_eq!(s.read(&link.new).unwrap().unwrap(), b"same.txt");
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn commits_one_at_a_time() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        git(r, &["init", "-q"]);
        write(&r.join("a.txt"), "a\n");
        write(&r.join("c.txt"), &"c\n".repeat(20));
        git(r, &["add", "."]);
        git(r, &["commit", "-qm", "base"]);
        git(r, &["switch", "-qc", "feature"]);
        write(&r.join("a.txt"), "a changed\n");
        git(r, &["commit", "-qam", "change a\n\nwith a body"]);
        write(&r.join("b.txt"), "new\n");
        git(r, &["mv", "c.txt", "moved.txt"]);
        git(r, &["add", "."]);
        git(r, &["commit", "-qm", "add b, move c"]);
        // Uncommitted work isn't in any commit.
        write(&r.join("a.txt"), "dirty\n");

        let s = Source::git(r, None).unwrap();
        let commits = s.commits().unwrap();
        let summaries: Vec<&str> = commits.iter().map(|c| c.summary.as_str()).collect();
        assert_eq!(summaries, ["change a", "add b, move c"]);
        let first = s.commit(commits[0].id).unwrap();
        assert_eq!(kinds(&first), [("a.txt".to_string(), Kind::Modified)]);
        let new = first.read(&first.changes[0].new).unwrap().unwrap();
        assert_eq!(
            new, b"a changed\n",
            "the commit's content, not the worktree's"
        );
        let second = s.commit(commits[1].id).unwrap();
        assert_eq!(
            kinds(&second),
            [
                ("b.txt".to_string(), Kind::Added),
                ("moved.txt".to_string(), Kind::Renamed)
            ]
        );
        // Without git there are no commits.
        let (o, n) = (r.join("a.txt"), r.join("b.txt"));
        assert!(Source::diff(&o, &n).unwrap().commits().is_err());
    }

    fn kinds(s: &Source) -> Vec<(String, Kind)> {
        s.changes
            .iter()
            .map(|c| (c.path.clone(), c.kind()))
            .collect()
    }

    #[test]
    fn since_a_snapshot() {
        let d = tempfile::tempdir().unwrap();
        let (o, n) = (d.path().join("o"), d.path().join("n"));
        write(&o.join("same.txt"), "base\n");
        write(&n.join("same.txt"), "reviewed\n");
        write(&o.join("moved.txt"), "base\n");
        write(&n.join("moved.txt"), "changed again\n");
        write(&n.join("new.txt"), "added after review\n");
        write(&o.join("back.txt"), "base\n");
        write(&n.join("back.txt"), "base\n");
        let s = Source::diff(&o, &n).unwrap();
        let snapshot: HashMap<String, Vec<u8>> = [
            ("same.txt", "reviewed\n"),
            ("moved.txt", "reviewed\n"),
            // Changed when reviewed, back to the base now.
            ("back.txt", "reviewed\n"),
        ]
        .into_iter()
        .map(|(p, c)| (p.to_string(), c.as_bytes().to_vec()))
        .collect();
        let since = s.since(&snapshot).unwrap();
        assert_eq!(
            kinds(&since),
            [
                ("back.txt".to_string(), Kind::Modified),
                ("moved.txt".to_string(), Kind::Modified),
                ("new.txt".to_string(), Kind::Added)
            ]
        );
        let c = &since.changes[1];
        assert_eq!(since.read(&c.old).unwrap().unwrap(), b"reviewed\n");
        assert_eq!(since.read(&c.new).unwrap().unwrap(), b"changed again\n");
        let back = &since.changes[0];
        assert_eq!(since.read(&back.new).unwrap().unwrap(), b"base\n");
    }

    #[test]
    fn git_branch_vs_base() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        git(r, &["init", "-q"]);
        for f in ["a", "b", "c", "d", "e", "mv1", "mv2", "run.sh"] {
            write(&r.join(format!("{f}.txt")), &format!("{f}\n").repeat(20));
        }
        write(&r.join(".gitattributes"), "gen/** linguist-generated\n");
        git(r, &["add", "."]);
        git(r, &["commit", "-qm", "base"]);
        git(r, &["switch", "-qc", "feature"]);
        write(&r.join("a.txt"), "a committed\n");
        git(r, &["mv", "mv1.txt", "renamed-committed.txt"]);
        write(&r.join("added.txt"), "new\n");
        write(&r.join("gen/out.rs"), "// generated\n");
        write(&r.join("Cargo.lock"), "lock\n");
        git(r, &["add", "."]);
        git(r, &["commit", "-qm", "work"]);
        // main moves on after the branch: not part of the review.
        git(r, &["switch", "-q", "main"]);
        write(&r.join("e.txt"), "main moved\n");
        git(r, &["commit", "-qam", "main moves"]);
        git(r, &["switch", "-q", "feature"]);
        write(&r.join("b.txt"), "b staged\n");
        git(r, &["add", "b.txt"]);
        write(&r.join("c.txt"), "c unstaged\n");
        fs::remove_file(r.join("d.txt")).unwrap();
        fs::rename(r.join("mv2.txt"), r.join("renamed-worktree.txt")).unwrap();
        write(&r.join("untracked/new.txt"), "u\n");
        chmod_x(&r.join("run.sh.txt"));
        write(&r.join(".gitignore"), "*.log\n");
        write(&r.join("x.log"), "ignored\n");

        let s = Source::git(r, None).unwrap();
        assert_eq!(
            summary(&s),
            [
                "Added .gitignore",
                "Added Cargo.lock",
                "Modified a.txt",
                "Added added.txt",
                "Modified b.txt",
                "Modified c.txt",
                "Deleted d.txt",
                "Added gen/out.rs",
                "Renamed renamed-committed.txt <- mv1.txt",
                "Renamed renamed-worktree.txt <- mv2.txt",
                "ModeChanged run.sh.txt",
                "Added untracked/new.txt",
            ]
        );
        let g = s.git.as_ref().unwrap();
        assert_eq!(g.branch.as_deref(), Some("feature"));
        assert_eq!(g.base, "main");
        let a = &s.changes[2];
        assert_eq!(
            s.read(&a.old).unwrap().unwrap(),
            "a\n".repeat(20).as_bytes()
        );
        assert_eq!(s.read(&a.new).unwrap().unwrap(), b"a committed\n");
        let collapsed: Vec<_> = s
            .changes
            .iter()
            .filter(|c| s.collapsed(c))
            .map(|c| c.path.as_str())
            .collect();
        assert_eq!(collapsed, ["Cargo.lock", "gen/out.rs"]);
    }

    #[test]
    fn binary() {
        assert!(is_binary(b"\x89PNG\0\0"));
        assert!(!is_binary("héllo\n".as_bytes()));
    }

    /// A repo on `main` with one commit, then `feature` checked out.
    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        git(r, &["init", "-q"]);
        write(&r.join("a.txt"), "a\n");
        git(r, &["add", "."]);
        git(r, &["commit", "-qm", "base"]);
        git(r, &["switch", "-qc", "feature"]);
        d
    }

    #[test]
    fn git_nothing_to_review() {
        let d = repo();
        assert!(Source::git(d.path(), None).unwrap().changes.is_empty());
        // Changed and then changed back is nothing, too.
        write(&d.path().join("a.txt"), "x\n");
        write(&d.path().join("a.txt"), "a\n");
        assert!(Source::git(d.path(), None).unwrap().changes.is_empty());
    }

    #[test]
    fn git_added_then_deleted_and_staged_then_reverted() {
        let d = repo();
        let r = d.path();
        write(&r.join("tmp.txt"), "t\n");
        git(r, &["add", "tmp.txt"]);
        fs::remove_file(r.join("tmp.txt")).unwrap();
        write(&r.join("a.txt"), "staged\n");
        git(r, &["add", "a.txt"]);
        write(&r.join("a.txt"), "a\n");
        assert_eq!(
            summary(&Source::git(r, None).unwrap()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn git_from_a_subfolder_and_paths_relative_to_the_worktree() {
        let d = repo();
        let r = d.path();
        write(&r.join("sub/deep/x.rs"), "x\n");
        let s = Source::git(&r.join("sub/deep"), None).unwrap();
        assert_eq!(summary(&s), ["Added sub/deep/x.rs"]);
        assert_eq!(s.root, r.canonicalize().unwrap());
    }

    #[test]
    fn git_base_choice() {
        let d = repo();
        let r = d.path();
        git(r, &["commit", "-q", "--allow-empty", "-m", "second"]);
        git(r, &["branch", "-q", "other"]);
        write(&r.join("b.txt"), "b\n");
        git(r, &["add", "."]);
        git(r, &["commit", "-qm", "third"]);
        // Default base: main.
        assert_eq!(summary(&Source::git(r, None).unwrap()), ["Added b.txt"]);
        // A given base wins.
        let s = Source::git(r, Some("other")).unwrap();
        assert_eq!(s.git.as_ref().unwrap().base, "other");
        assert_eq!(summary(&s), ["Added b.txt"]);
        // A base that doesn't exist is an error naming it.
        let err = Source::git(r, Some("nope")).err().unwrap().to_string();
        assert!(err.contains("nope"), "{err}");
    }

    #[test]
    fn git_prefers_origins_default_branch() {
        let up = repo();
        git(up.path(), &["switch", "-q", "main"]);
        git(up.path(), &["branch", "-qm", "main", "trunk"]);
        let d = tempfile::tempdir().unwrap();
        let clone = d.path().join("c");
        git(d.path(), &["clone", "-q", up.path().to_str().unwrap(), "c"]);
        git(&clone, &["switch", "-qc", "work"]);
        write(&clone.join("n.txt"), "n\n");
        let s = Source::git(&clone, None).unwrap();
        assert_eq!(s.git.as_ref().unwrap().base, "origin/trunk");
        assert_eq!(summary(&s), ["Added n.txt"]);
    }

    #[test]
    fn git_detached_head_and_no_shared_history() {
        let d = repo();
        let r = d.path();
        git(r, &["checkout", "-q", "--detach"]);
        write(&r.join("d.txt"), "d\n");
        let s = Source::git(r, None).unwrap();
        assert_eq!(s.git.as_ref().unwrap().branch, None);
        assert_eq!(summary(&s), ["Added d.txt"]);
        // An orphan branch shares no history with main.
        git(r, &["checkout", "-q", "--orphan", "lonely"]);
        git(r, &["commit", "-qm", "alone"]);
        let err = Source::git(r, None).err().unwrap().to_string();
        assert!(err.contains("shares no history"), "{err}");
    }

    #[test]
    fn git_symlinks_binary_and_ignored_dirs() {
        let d = repo();
        let r = d.path();
        std::os::unix::fs::symlink("a.txt", r.join("link")).unwrap();
        fs::write(r.join("img.bin"), b"\x00\x01\x02").unwrap();
        write(&r.join(".gitignore"), "build/\n");
        write(&r.join("build/out.o"), "o\n");
        let s = Source::git(r, None).unwrap();
        assert_eq!(
            summary(&s),
            ["Added .gitignore", "Added img.bin", "Added link"]
        );
        let link = s.changes.iter().find(|c| c.path == "link").unwrap();
        assert_eq!(link.new_mode, Some(Mode::Symlink));
        assert_eq!(s.read(&link.new).unwrap().unwrap(), b"a.txt");
        let bin = s.changes.iter().find(|c| c.path == "img.bin").unwrap();
        assert!(is_binary(&s.read(&bin.new).unwrap().unwrap()));
    }

    #[test]
    fn git_outside_a_repo() {
        let d = tempfile::tempdir().unwrap();
        let err = Source::git(d.path(), None).err().unwrap().to_string();
        assert!(err.contains("not in a git repository"), "{err}");
    }

    #[test]
    fn folders_edge_cases() {
        let d = tempfile::tempdir().unwrap();
        let (old, new) = (d.path().join("old"), d.path().join("new"));
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        // Identical folders: nothing.
        assert!(Source::diff(&old, &new).unwrap().changes.is_empty());
        // Two deleted files with the same content and one added: one rename, one deletion.
        write(&old.join("x1"), "same\n");
        write(&old.join("x2"), "same\n");
        write(&new.join("y"), "same\n");
        // Empty folders aren't changes; a nested .gitignore applies below it.
        fs::create_dir_all(new.join("empty")).unwrap();
        write(&new.join("sub/.gitignore"), "*.tmp\n");
        write(&new.join("sub/keep.rs"), "k\n");
        write(&new.join("sub/skip.tmp"), "s\n");
        let s = Source::diff(&old, &new).unwrap();
        let got = summary(&s);
        assert_eq!(got.len(), 4, "{got:?}");
        assert!(got.contains(&"Added sub/.gitignore".to_string()));
        assert!(got.contains(&"Added sub/keep.rs".to_string()));
        assert_eq!(
            got.iter()
                .filter(|g| g.starts_with("Renamed y <- x"))
                .count(),
            1,
            "{got:?}"
        );
        assert_eq!(
            got.iter().filter(|g| g.starts_with("Deleted x")).count(),
            1,
            "{got:?}"
        );
    }

    #[test]
    fn diff_needs_two_paths_of_the_same_kind() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f");
        write(&f, "x\n");
        assert!(Source::diff(&f, &d.path().join("missing")).is_err());
        let err = Source::diff(&f, d.path()).err().unwrap().to_string();
        assert!(err.contains("two files or two folders"), "{err}");
    }

    #[test]
    fn generated_files() {
        let c = |p: &str| Change {
            path: p.into(),
            old_path: None,
            old: Content::None,
            new: Content::None,
            old_mode: None,
            new_mode: None,
            same_content: false,
        };
        for p in [
            "Cargo.lock",
            "web/package-lock.json",
            "go.sum",
            "a/b/app.min.js",
            "x.css.map",
        ] {
            assert!(c(p).looks_generated(), "{p}");
        }
        for p in ["Cargo.toml", "lock.rs", "min.js", "src/main.rs"] {
            assert!(!c(p).looks_generated(), "{p}");
        }
    }
}
