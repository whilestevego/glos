# Reviewing a folder that isn't in git

**Status:** Draft (2026-11-06). This answers the overview's open question on a folder with no
second copy, in `completed/plan-overview.md`.

glos can review a git branch, git revisions, or two files or folders. A single folder that isn't
in git has nothing to compare against, so glos can't review it. An agent working in a scratch
folder leaves you with nothing to review.

This plan takes a snapshot of the folder when its review starts. From then on, the snapshot is
the old side and the folder on disk is the new side, so the review shows everything that changed
since it started.

## What it looks like

- `glos` in a folder that isn't in git asks whether to start a review of it from now on. The
  agent's way in is `request_review` with the folder, and the CLI's is
  `glos open-review folder <dir>`.
- At first there's nothing to review. As the agent changes files, they appear as changes, with
  added and deleted files too.
- "Since your last review" works as it does for other reviews, against the round's snapshot.
- `:rebaseline` takes a new snapshot of the folder and makes it the old side, for when you've
  accepted everything so far. It asks first, and threads on the old changes become outdated.

## How it works

- **The snapshot uses what's already there.** File contents are stored once by hash in `blobs`
  (`src/store.rs`), and `snapshot_files` records which file had which content in a round. The
  baseline is stored the same way, as round 0 of the review, with the verdict `baseline`. Every
  place that lists rounds, such as "since your last review", the dashboard and `glos show`, skips
  round 0.
- **The old side is `Content::Bytes`.** `src/source.rs` already has `Content::Bytes` for a file as
  it was at your last submit. A folder review builds its old side from the baseline's blobs, and
  its new side from the files on disk.
- **Which files are included.** glos walks the folder with `src/source.rs` `walk`, which a
  folder diff already uses. It honours `.gitignore` even outside git, includes dotfiles, and
  skips `.git`. The snapshot also skips binary files over 1 MB.
- **Limits.** A folder with more than 20,000 files or 200 MB of text is refused, with a message
  that says which limit it hit, so a mistaken `glos` in a home folder doesn't copy it into the
  database.
- **Its key.** The review's `source_kind` is `folder`, and its `source_key` is the folder's
  canonical path.
- **`glos gc`** removes a baseline's blobs with the review, as it does for rounds.

## Steps

1. Take the baseline, store it as round 0, and build a `Source` from it and the folder.
2. Starting it: `glos` in a non-git folder, `glos open-review folder`, and `request_review`.
3. `:rebaseline`.
4. The README's "What it can review" section.

## Tests

- A folder review shows an edited, an added and a deleted file after the agent changes them.
- Ignored files and big binaries are left out of the baseline.
- A folder over the limits is refused with the right message.
- `:rebaseline` empties the review, and outdates threads on the old changes.
- `glos gc` deletes the baseline's blobs with an archived review.

## Open

- Should glos ask before starting a folder review, or start one silently when run in a folder
  that isn't in git?

## Relevant code

- `src/source.rs` `Source`, `Content::Bytes` and the `ignore` walk.
- `src/store.rs` `blobs`, `rounds` and `snapshot_files`.
- `src/review.rs`, for the key, `open-review` and `gc`.
- `src/main.rs`, for what plain `glos` does outside git.
- `src/mcp.rs` `request_review`.
