# Plans

These are the design and build plans for glos. Each plan stands on its own. It has its own
status, scope and steps, and a reader can start work on it without the other plans.

Two files here are not plans. [DECISIONS.md](DECISIONS.md) is the log of design decisions. It
gives the options that were considered and the reason for each choice. [KEYMAP.md](KEYMAP.md)
is the keymap checklist, and `cargo xtask keymap` writes it. Do not edit it by hand.

## Names and layout

- Each plan is one file with the name `plan-<topic>.md`. The topic is short and in kebab-case,
  such as `plan-diff-highlight.md`.
- File names have no milestone or phase numbers. The order and the dependencies go inside the
  plans, so that you can change the order without a rename.
- Each plan has a `**Status:**` line under its title. The statuses are `Draft`,
  `Ready to build`, `Blocked` (with what it waits for), `In progress` (with the branch),
  `Implemented (YYYY-MM-DD) in PR #N` and `Abandoned (YYYY-MM-DD)`.
- [completed/plan-overview.md](completed/plan-overview.md) is the master plan and the map of
  glos. Its milestones are built, so it is in `completed/`. The work that it lists without a
  plan of its own stays there.
- When a plan is delivered, move it into `completed/` and update its status line. Do not change
  the rest of the file, apart from its "As built" section.
- When a plan is dropped, move it into `abandoned/`. Change its status line to give the date,
  the reason and what would bring it back. Keep the body, because the research in it is the
  reason to keep the file. If a part of it is still necessary, move that part into a live plan,
  and name each plan in the other.
- Update this index each time a plan is added, moved or changes status.

## Active plans

| Plan | Status |
|---|---|
| [plan-language-servers.md](plan-language-servers.md) | Draft. Diagnostics, go to definitions and references, hover and symbol pickers. |
| [plan-in-place-editing.md](plan-in-place-editing.md) | Draft. Edit any file on disk with Helix's keys, and merge the changes of an agent into unsaved edits. |
| [plan-channels.md](plan-channels.md) | Blocked until Claude Code releases channels. Sends your review into the session of the agent. |
| [plan-review-identity.md](plan-review-identity.md) | Draft. Reviews follow renamed branches, and the key of a detached HEAD is its start commit. |
| [plan-folder-snapshot.md](plan-folder-snapshot.md) | Draft. Review a folder outside git against a snapshot from the start of the review. |
| [plan-code-quality.md](plan-code-quality.md) | In progress on `build/code-quality`. Lints, formatting, git hooks, a prose lint and `cargo xtask ci`. |

Remappable keys, from the "After the MVP" list in
[completed/plan-overview.md](completed/plan-overview.md), have no plan yet.

## Abandoned

| Plan | Why |
|---|---|
| [abandoned/plan-removed-lines.md](abandoned/plan-removed-lines.md) | 2026-11-08. The whole-file view was removed, and the unified view already shows removed lines. |

## Completed

| Plan | Delivered |
|---|---|
| [completed/plan-review-menu.md](completed/plan-review-menu.md) | 2026-11-09, PRs #11 and #18. The review commands are under `'` and the thread commands are on `]r`, so Helix's keys are free. |
| [completed/plan-old-side-comments.md](completed/plan-old-side-comments.md) | 2026-11-09, PR #17. Comment on removed lines in the unified and side-by-side views. |
| [completed/plan-reanchoring.md](completed/plan-reanchoring.md) | 2026-11-08, PR #15. Threads do not jump to far copies or to copies of short lines, and a small edit keeps them in place. |
| [completed/plan-all-files.md](completed/plan-all-files.md) | 2026-11-08, PRs #12, #13 and #14. All changed files in one scroll, by default, in each view. Search and jumps go across files. |
| [completed/plan-default-theme.md](completed/plan-default-theme.md) | 2026-11-06, PR #10. catppuccin_mocha is the default theme. In 256 colours, the diff tints are green and red, not grey. |
| [completed/plan-diff-highlight.md](completed/plan-diff-highlight.md) | 2026-11-06, PR #9. Changed lines get a tint mixed from the theme, and changed words get a stronger tint. |
| [completed/plan-keymap-contexts.md](completed/plan-keymap-contexts.md) | 2026-11-06, PR #8. One keymap binds the keys of each context, and `Ctrl-G` names the context in its border. |
| [completed/plan-tree.md](completed/plan-tree.md) | 2026-11-06, PR #7. Tree folders open and close, and `/` searches the names. |
| [completed/plan-delete-comment.md](completed/plan-delete-comment.md) | 2026-11-06, PR #6. `space M` deletes your latest unsent comment. |
| [completed/plan-mouse.md](completed/plan-mouse.md) | 2026-11-06, PR #5. A click focuses the panel under it, and the wheel scrolls the panel under the pointer. |
| [completed/plan-components.md](completed/plan-components.md) | 2026-11-05, PR #3. The input of `App` is split into components. `plan-keymap-contexts.md` later reversed its choice to let components match their own keys. |
| [completed/plan-folding.md](completed/plan-folding.md) | 2026-11-04, PR #1. Unchanged code folds around changes, and you can make your own folds. |
| [completed/plan-overview.md](completed/plan-overview.md) | 2026-11-04. Milestones M0 to M5 are built. It stays the map. |
