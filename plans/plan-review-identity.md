# Branch renames and detached HEAD

**Status:** Draft (2026-11-06). This answers the overview's open question on branch renames and
detached HEAD, in `completed/plan-overview.md`.

A git review is found by its key: the shared git folder, the worktree and the branch name
(`src/review.rs`, where `source_key` is built). That breaks in two cases.

- **A renamed branch loses its review.** `Review::source` sees the worktree is on another branch
  and refuses: "review X is of branch old, but … is on new now". `archive_finished` then finds no
  `refs/heads/old` and archives the review as if the branch were deleted.
- **Detached HEAD shares one review.** Every review started in detached HEAD in a worktree is keyed
  as `(detached)`, so reviews of unrelated commits are treated as one.

## Following a rename

When a review's branch is missing, glos looks for where it went before it archives the review.

1. **The reflog.** `git branch -m old new` writes "Branch: renamed refs/heads/old to
   refs/heads/new" into the new branch's reflog. If a branch's reflog has that entry, it's the
   renamed branch.
2. **The commits.** Without a reflog entry, a branch in the same worktree whose tip contains the
   last commit glos saw for the review counts, if no other review has it. The last commit is
   `head_commit` of the review's latest round.

If either finds the branch, glos updates the review's `branch` and `source_key`, and the status
line and `glos list` say "followed the rename from old to new". If neither does, the review is
archived as today.

This runs in `archive_finished`, and in `Review::source` before it refuses.

## Detached HEAD

A review started in detached HEAD is keyed by the commit HEAD was on when it started, as
`(detached <commit>)`. Opening glos in detached HEAD finds the review in that worktree whose
start commit is HEAD or an ancestor of it. So new commits on top stay in the same review, and
checking out an unrelated commit starts a new one. If several reviews match, the one with the
nearest start commit wins.

`archive_finished` skips detached reviews today. With the start commit, it can archive one once
its commits are merged into the base, as it does for a branch.

Existing `(detached)` reviews keep working. They're treated as started at their first round's
`head_commit`, or left alone if they have no rounds.

## Steps

1. Follow renames by the reflog, then by the commits, in `archive_finished` and
   `Review::source`.
2. Key detached reviews by their start commit, find them by ancestry, and archive them when
   they're merged.
3. Remove the `ponytail:` comment in `src/review.rs` that points at the open question.

## Tests

Each test builds a small repository with `gix` or the `git` command in a temporary folder, as the
tests in `src/review.rs` do now.

- `git branch -m` keeps the review, with its threads, under the new name.
- A branch recreated under another name from the same commits, with no reflog entry, keeps the
  review.
- A deleted branch with no successor is still archived.
- Two detached commits on unrelated histories get two reviews, and a commit on top of a detached
  review's start stays in that review.
- An existing `(detached)` review from an older database still opens.

## Relevant code

- `src/review.rs`: `source_key`, `Review::source`, `archive_finished` and `branch_over`.
- `src/source.rs` `Source::git`, where the branch is read.
- `src/store.rs`, if the start commit needs its own column rather than living in the key.
