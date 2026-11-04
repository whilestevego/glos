# Spike 3: branch vs base with gix — works

gix 0.90.0, imara-diff 0.2.0. `cargo run -- [path] [base]`.

## How

One `repo.status()` pass with `.head_tree(merge_base_tree)` compares merge-base tree → index →
worktree, so committed, staged, unstaged and untracked changes come out of one call:

- `Item::TreeIndex` (merge-base vs index): committed + staged. `Change::Rewrite` gives renames
  (`TrackRenames::AsConfigured`, i.e. git's `diff.renames`).
- `Item::IndexWorktree`: unstaged edits, worktree deletions, untracked files
  (`UntrackedFiles::Files`), and renames on disk without `git mv`
  (`index_worktree_rewrites(Some(default))`).

Per path: old = blob at the merge-base tree (`lookup_entry_by_path`), new = file on disk. Then
imara-diff (Histogram + `postprocess_lines`) for the line diff. A path changed and reverted gives
equal old/new and is dropped.

Base: `refs/remotes/origin/HEAD`'s target, else `main`, `master`, `origin/main`, `origin/master`.
`repo.merge_base(head, base)`. Review key: `repo.common_dir()` (canonicalized; for a linked
worktree it's `.git/worktrees/x/../..`) + `repo.workdir()` + `repo.head_name()`.

## Verified (scratch repo, every kind at once)

Committed modify/add/rename, `main` advanced after branching (correctly excluded via
merge-base), staged edit, unstaged edit, `rm` without `git rm`, `mv` without `git mv` (detected
as rename), untracked files incl. in a new dir, `.gitignore` honoured, linked worktree (own
branch and edits, same common dir).

## Speed

- Small repo: 13 ms total (debug build).
- This repo with 1,063 untracked build files: 24 s debug, 1.2 s release. gix in debug is very
  slow: M1 needs `[profile.dev.package."*"] opt-level = 2` (or 3).
- Untracked files are all read and diffed here, binaries included. M1: detect binary (NUL in the
  first 8 KB, as git) before diffing, and diff lazily per file as the plan says; only paths and
  kinds are needed for the first screen.

## Not covered (M1)

- Mode changes and symlinks: available on both `Change` (`entry_mode`) and worktree entries; not
  printed here.
- Detached HEAD: `head_name()` is `None`; key decision still open (PLAN open questions).
- `linguist-generated` collapse: needs gix attributes lookup (`repo.attributes(...)`), untested.
- Per-commit scope: commit walk (`rev_walk`) untested; tree-to-tree diff is the same machinery.
- No fallback to shelling out to git was needed.
