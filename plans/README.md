# Plans

These are the design and build plans for glos. Each plan stands on its own. It carries its own
status, scope and steps, and someone can pick it up without reading the others.

Two files here are not plans. [DECISIONS.md](DECISIONS.md) is the log of design decisions, with
the options considered and why one won. [KEYMAP.md](KEYMAP.md) is the keymap checklist, which
`cargo xtask keymap` generates. Don't edit it by hand.

## Naming and layout

- Each plan is one file named `plan-<topic>.md`, where the topic is short and in kebab-case, such
  as `plan-diff-highlight.md`.
- File names carry no milestone or phase numbers. Order and dependencies go inside the plans, so
  plans can be reordered without renaming them.
- Every plan opens with a `**Status:**` line under its title. The statuses in use are `Draft`,
  `Ready to build`, `Blocked` (with what it waits for), `In progress` (with the branch),
  `Implemented (YYYY-MM-DD) in PR #N` and `Abandoned (YYYY-MM-DD)`.
- [completed/plan-overview.md](completed/plan-overview.md) is the master plan and the map of
  glos. Its milestones are built, so it lives in `completed/`. Work it lists that has no plan of
  its own yet stays there.
- When a plan is delivered, move it into `completed/` and update its status line. Leave the rest
  of the file as it is, apart from its "As built" section.
- When a plan is dropped, move it into `abandoned/`. Rewrite its status line to give the date, the
  reason, and what would bring it back. Keep the body, because the research is the reason to keep
  the file. If anything from it is still wanted, move it into a live plan and name each plan in
  the other.
- Update this index whenever a plan is added, moved or changes status.

## Active plans

| Plan | Status |
|---|---|
| [plan-review-menu.md](plan-review-menu.md) | In progress. Review commands move under `'` so Helix gets its keys back. Step 1 builds the menu. Steps 2 and 3 remove the old keys when Helix needs them. |
| [plan-language-servers.md](plan-language-servers.md) | Draft. Diagnostics, going to definitions and references, hover and symbol pickers. Needs step 2 of `plan-review-menu.md`. |
| [plan-in-place-editing.md](plan-in-place-editing.md) | Draft. Edit any file on disk with Helix's keys, and merge an agent's changes into unsaved edits. Needs step 3 of `plan-review-menu.md`. |
| [plan-channels.md](plan-channels.md) | Blocked until Claude Code releases channels. Pushes your review into the agent's session. |
| [plan-old-side-comments.md](plan-old-side-comments.md) | Draft. Comment on removed lines in the unified and side-by-side views. |
| [plan-review-identity.md](plan-review-identity.md) | Draft. Reviews follow renamed branches, and detached HEAD is keyed by its start commit. |
| [plan-folder-snapshot.md](plan-folder-snapshot.md) | Draft. Review a folder outside git against a snapshot taken when the review starts. |

Remappable keys, from the overview's "After the MVP" list in
[completed/plan-overview.md](completed/plan-overview.md), has no plan yet.

## Abandoned

| Plan | Why |
|---|---|
| [abandoned/plan-removed-lines.md](abandoned/plan-removed-lines.md) | 2026-11-08. The whole-file view was removed, and the unified view already shows removed lines. |

## Completed

| Plan | Delivered |
|---|---|
| [completed/plan-reanchoring.md](completed/plan-reanchoring.md) | 2026-11-08, PR #15. Threads don't jump to far copies or copies of short lines, and a small edit keeps them. |
| [completed/plan-all-files.md](completed/plan-all-files.md) | 2026-11-08, PRs #12, #13 and #14. Every changed file in one scroll, by default, in each view, with search and jumps that cross files. |
| [completed/plan-default-theme.md](completed/plan-default-theme.md) | 2026-11-06, PR #10. catppuccin_mocha is the default theme, and 256-colour diff tints are green and red instead of grey. |
| [completed/plan-diff-highlight.md](completed/plan-diff-highlight.md) | 2026-11-06, PR #9. Changed lines get a tint mixed from the theme, and changed words get a stronger one. |
| [completed/plan-keymap-contexts.md](completed/plan-keymap-contexts.md) | 2026-11-06, PR #8. One keymap binds every context's keys, and `Ctrl-G` names the context in its border. |
| [completed/plan-tree.md](completed/plan-tree.md) | 2026-11-06, PR #7. Tree folders open and close, and `/` searches the names. |
| [completed/plan-delete-comment.md](completed/plan-delete-comment.md) | 2026-11-06, PR #6. `space M` deletes your latest unsent comment. |
| [completed/plan-mouse.md](completed/plan-mouse.md) | 2026-11-06, PR #5. A click focuses the panel it lands on, and the wheel scrolls what's under the pointer. |
| [completed/plan-components.md](completed/plan-components.md) | 2026-11-05, PR #3. `App`'s input is split into components. Its choice to let components match their own keys was later reversed by `plan-keymap-contexts.md`. |
| [completed/plan-folding.md](completed/plan-folding.md) | 2026-11-04, PR #1. Unchanged code folds around changes, and you can make your own folds. |
| [completed/plan-overview.md](completed/plan-overview.md) | 2026-11-04. Milestones M0 to M5 are built. It stays the map. |
