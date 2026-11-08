# Removed lines in the whole-file view

**Status:** Draft (2026-11-06).

The whole-file view shows the new file, with changes marked in the gutter. When a change deleted
or replaced lines, you can't see what was there without switching to the unified or
side-by-side view. This plan shows the removed lines in the whole-file view, always, at the change
that removed them, so you see what was there and what replaced it in one place.

## What it looks like

- Each run of removed lines shows right above the lines that replaced it, or where it was if
  nothing replaced it. That's the order the unified view uses.
- Removed lines have the `diff.minus.line` background, the changed-word tint from
  `completed/plan-diff-highlight.md`, and the old side's syntax highlighting. The gutter shows `-` and the old line number, as the unified view does.
- Folding counts them. A fold band that hides a change also hides its removed lines.

## How it works

The removed lines are virtual lines, not lines of the document. Helix draws virtual text through
`TextAnnotations` and `LineAnnotation`, which `glos-helix` already exports in `layout`. The
document stays the new file as it is, so three things keep working:

- Motions, search and selections act on the real file. `j` steps over the removed lines, as it
  steps over Helix's inline diagnostics.
- In-place editing (`plan-in-place-editing.md`) edits the real file. The removed lines move with
  their change as you edit.
- Line numbers, threads and comments stay on the new side.

Since the cursor never lands on a removed line, you comment on one in the unified or side-by-side
view, as `plan-old-side-comments.md` describes.

A removed line's text and highlighting come from the old side's `File`, which `src/ui/diffview.rs`
already builds for the other views. Its changed words come from `src/diff_words.rs` `changed`,
which already works out both sides.

In the all-files mode of `completed/plan-all-files.md`, a file's row count includes its removed lines, so
the scroll stays exact.

## Steps

1. Draw the removed lines as line annotations in `src/ui/view.rs` `draw`, with their gutter,
   background and highlighting.
2. Make folding (`src/ui/diffview.rs` `rows` and `hidden`) count them.
3. Update the README's description of the whole-file view.

## Tests

- A screen snapshot of a file with a replaced block, a deleted block and an added block.
- `j` and `k` skip the removed lines, and `/` doesn't find text that's only in them.
- A colour test that removed lines have the `diff.minus.line` background and their changed words
  the `diff.minus.word` one.
- A fold around a change hides its removed lines, and opening it shows them.
- The perf run in `src/ui/tests/perf.rs` stays within its budget.

## Open

- Should a key or setting hide them for a while, for a file with large rewrites? You chose to
  always show them, so the plan has no switch.

## Relevant code

- `src/ui/view.rs` `draw` and `text_format`.
- `crates/glos-helix/src/lib.rs` `layout`.
- `src/ui/diffview.rs` `rows`, `hidden` and `Folds`.
- `src/diff_words.rs`.
