//! M0 spike 3: branch vs base with gix, uncommitted and untracked files included.
//!
//! `gitdiff [path] [base]`: finds the repo, the base (origin's default branch, else main/master,
//! or the given ref), the merge-base with HEAD, then asks gix status to compare that merge-base
//! tree → index → worktree in one pass. Each changed path gets its old text (merge-base blob) and
//! new text (file on disk), diffed with imara-diff.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result, anyhow};
use gix::{
    bstr::{BString, ByteSlice},
    diff::index::Change,
    status::{Item, UntrackedFiles, index_worktree},
};
use imara_diff::{Algorithm, Diff, InternedInput};

/// One changed file: the old path (at the merge-base), the new path (in the worktree).
#[derive(Debug, Default)]
struct Entry {
    old_path: Option<BString>,
    untracked: bool,
}

fn find_base(repo: &gix::Repository, given: Option<&str>) -> Result<gix::ObjectId> {
    let candidates: Vec<String> = match given {
        Some(r) => vec![r.into()],
        None => {
            let mut v = vec![];
            // origin/HEAD is a symbolic ref to origin's default branch.
            if let Ok(Some(r)) = repo.try_find_reference("refs/remotes/origin/HEAD") {
                if let Some(name) = r.target().try_name() {
                    v.push(name.as_bstr().to_string());
                }
            }
            v.extend(["main", "master", "origin/main", "origin/master"].map(String::from));
            v
        }
    };
    for c in &candidates {
        if let Ok(id) = repo.rev_parse_single(c.as_str()) {
            return Ok(id.detach());
        }
    }
    Err(anyhow!("no base found among {candidates:?}"))
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().map(PathBuf::from).unwrap_or_else(|| ".".into());
    let t0 = Instant::now();

    let repo = gix::discover(&path).context("not a git repo")?;
    let workdir = repo.workdir().context("bare repo")?.to_owned();
    let head = repo.head_commit()?;
    let branch = repo.head_name()?.map(|n| n.shorten().to_string());
    let base = find_base(&repo, args.get(1).map(String::as_str))?;
    let mb = repo.merge_base(head.id, base)?.context("no merge-base")?.detach();
    let mb_tree = repo.find_commit(mb)?.tree_id()?.detach();

    println!("common dir: {}", repo.common_dir().canonicalize()?.display());
    println!("worktree:   {}", workdir.display());
    println!("branch:     {branch:?}  head {}", head.id.to_hex_with_len(8));
    println!("base:       {}  merge-base {}", base.to_hex_with_len(8), mb.to_hex_with_len(8));

    // Merge-base tree → index (committed + staged), index → worktree (unstaged + untracked).
    let mut files: BTreeMap<BString, Entry> = BTreeMap::new();
    let status = repo
        .status(gix::progress::Discard)?
        .head_tree(mb_tree)
        .untracked_files(UntrackedFiles::Files)
        .index_worktree_rewrites(Some(Default::default()))
        .tree_index_track_renames(gix::status::tree_index::TrackRenames::AsConfigured);
    for item in status.into_iter(Vec::<BString>::new())? {
        match item? {
            Item::TreeIndex(change) => match change {
                Change::Rewrite { source_location, location, .. } => {
                    files.entry(location.into_owned()).or_default().old_path =
                        Some(source_location.into_owned());
                }
                Change::Deletion { location, .. } => {
                    let loc = location.into_owned();
                    files.entry(loc.clone()).or_default().old_path.replace(loc);
                }
                other => {
                    let loc = other.location().to_owned();
                    files.entry(loc.clone()).or_default().old_path.get_or_insert(loc);
                }
            },
            Item::IndexWorktree(item) => match item {
                index_worktree::Item::Rewrite { source, dirwalk_entry, .. } => {
                    // A staged file moved on disk without `git mv`: carry the old side over.
                    let src = source.rela_path().to_owned();
                    let old = files.remove(&src).and_then(|e| e.old_path).unwrap_or(src);
                    files.entry(dirwalk_entry.rela_path).or_default().old_path = Some(old);
                }
                index_worktree::Item::DirectoryContents { entry, .. } => {
                    if entry.status == gix::dir::entry::Status::Untracked {
                        files.entry(entry.rela_path).or_default().untracked = true;
                    }
                }
                index_worktree::Item::Modification { rela_path, .. } => {
                    let e = files.entry(rela_path.clone()).or_default();
                    e.old_path.get_or_insert(rela_path);
                }
            },
        }
    }
    let t_status = t0.elapsed();

    let tree = repo.find_tree(mb_tree)?;
    for (new_path, e) in &files {
        let old = match &e.old_path {
            Some(p) => tree
                .lookup_entry_by_path(p.to_path_lossy())?
                .map(|entry| entry.object().map(|o| o.detach().data))
                .transpose()?,
            None => None,
        };
        let new = std::fs::read(workdir.join(Path::new(&new_path.to_path_lossy().into_owned()))).ok();
        if old == new && e.old_path.as_ref() == Some(new_path) {
            continue; // e.g. changed then reverted in the worktree
        }
        let kind = match (&old, &new) {
            (None, _) if e.untracked => "?",
            (None, _) => "A",
            (_, None) => "D",
            _ if e.old_path.as_ref() != Some(new_path) => "R",
            _ => "M",
        };
        let empty = Vec::new();
        let (o, n) = (old.as_ref().unwrap_or(&empty), new.as_ref().unwrap_or(&empty));
        let (o, n) = (String::from_utf8_lossy(o), String::from_utf8_lossy(n));
        let input = InternedInput::new(o.as_ref(), n.as_ref());
        let mut diff = Diff::compute(Algorithm::Histogram, &input);
        diff.postprocess_lines(&input);
        let from = match kind {
            "R" => format!("  (from {})", e.old_path.as_ref().unwrap()),
            _ => String::new(),
        };
        println!(
            "{kind} {new_path}{from}  +{} -{}",
            diff.count_additions(),
            diff.count_removals()
        );
    }
    eprintln!("status {:?}, total {:?}", t_status, t0.elapsed());
    Ok(())
}
