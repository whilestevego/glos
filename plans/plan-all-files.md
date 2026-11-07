# All files on one screen

**Status:** Draft (2026-11-06).

Today glos shows one file at a time. You move between files with `gn` and `gp`, the tree, or
`space g`. Search and the change keys (`]g`) stay inside the file on screen.

This plan adds an all-files mode. Every changed file is shown in one long scroll, one after
another, each under a header with its path, as GitHub's "Files changed" page and Zed's
multibuffer do. You scroll through the whole review, and search finds matches in every file.

## What it looks like

- **A mode, not a view.** The three views (whole file, unified and side by side) each work in
  both modes. In all-files mode, the whole-file view shows each file whole, with unchanged code
  folded as it is today. The unified view shows each file's hunks, and side by side shows each
  file's two sides.
- **It's the default.** glos opens in all-files mode. `[editor] all-files = false` in
  `config.toml` opens on one file instead.
- **One key switches modes.** `' v a` switches between all files and the file under
  the cursor, and the cursor stays on the same line. `:all` and `:one` do the same.
- **Each file has a header.** The header is one row with the path, the kind of change (added,
  deleted or renamed), `+added -removed`, and the open-thread count, as the tree shows them.
  `enter` on a header collapses or expands that file. Lockfiles and generated files start
  collapsed, as they do today.
- **Threads and comments work as they do now.** A thread shows under its line, in whichever
  file's section that line is.
- **The tree follows the cursor.** The tree marks the file the cursor is in. Clicking a file in
  the tree, or picking one with `space g`, moves the cursor to that file's header in all-files
  mode. `gn` and `gp` move to the next and previous file's header.
- **Search covers the review.** `/`, `?`, `n`, `N` and `*` search through every file in order,
  and wrap around at the end of the review. `]g` and `[g` move through every change.
- **Files outside the review stay alone.** A file opened for context with `space f` isn't part
  of the review, so it shows in one-file mode. `' v a` goes back to all files.
- **Filters and scopes apply.** `:filter` and `space o` decide which files are in the scroll.

## How it works

### One document per file, stacked

The views are already documents. `src/ui/diffview.rs` builds each file's unified or side-by-side
view as a `File` whose text is made of lines from both sides, with a `Row` per line. Helix's
motions, search and selections work on it because it's a normal `Rope`.

All-files mode stacks those per-file documents under their headers. It doesn't build one document
out of every file. The cursor is always inside one file's document, and keys work on that
document as they do today. Only the moves that leave a document need new code:

- `j` and `k`, the page keys and the mouse wheel continue into the next or previous file.
- `gg` and `G` go to the start of the first file and the end of the last.
- Search, `]g` and the thread keys continue into the next file when the current one has no more
  matches.
- `%` and other selections stay within one file. A selection can't span two files.

The alternative is one `Rope` holding every file, which would make every Helix command cross
files with no new code. It's rejected below, because it has to be rebuilt whenever any file
changes, a selection across files can't be commented on, and it can't be virtualized.

### Virtualized

A review can have hundreds of files of thousands of lines. All-files mode must open as fast as
one-file mode does today, and scroll smoothly through the whole review.

- **What's cheap is done for every file.** At startup, `src/ui/mod.rs` `tree_entries` already
  reads every changed file and diffs it, to count lines for the tree. Those hunks are kept
  rather than thrown away. With each file's hunks and line count, glos knows how many rows each
  file takes in any view, folds included, without parsing anything. So the scroll position, the
  scrollbar and `G` are exact from the start.
- **What's expensive is done on screen only.** Syntax highlighting, the changed-word ranges from
  `src/diff_words.rs`, and threads placed onto the file all wait until a file's rows come near
  the screen. glos keeps the documents for the files on screen and a screen above and below,
  and drops the rest when there are more than about 50.
- **Search doesn't build documents.** Search runs over each file's text, which is cheap, and only
  builds the document of the file it lands in.

`State` in `src/ui/mod.rs` already loads files lazily, with `files: Vec<Option<File>>` and
`App::load`. The new pieces are the per-file row counts, a list of sections (a header and a
file's document) with a scroll offset across them, and dropping documents that are off screen.
`Alt` holds one file's unified or side-by-side document today. In all-files mode it becomes one
per loaded file.

### Folds

Folds are kept per file and view already (`folds` in `App`, keyed by path and `ViewKind`), so
they carry over to all-files mode as they are. `zR` and `zM` act on the file under the cursor.

## Steps

1. **Keep every file's hunks and row counts** from `tree_entries`, and use them for the tree as
   today.
2. **All-files mode in the whole-file view.** Stack the sections with headers, scroll and move
   across them, load and drop documents, and switch modes with `' v a`, `:all` and `:one`.
   Add `[editor] all-files`.
3. **The unified and side-by-side views** in all-files mode.
4. **Search, `]g`, the thread keys, `gn` and `gp`** across files, and the tree following the
   cursor.
5. **Mouse:** clicks and the wheel across sections, and clicking a header.
6. **Make it the default**, and update the README and the overview's "Viewing the code" section
   in `completed/plan-overview.md`.

## Tests

- Screen snapshots of all-files mode in each view, with headers, folds and a collapsed file.
- `j` at the end of one file moves to the next, `G` reaches the last line of the last file, and
  `gg` comes back.
- `/` finds a match in a later file, `n` wraps from the last file to the first, and `]g` crosses
  files.
- `' v a` keeps the cursor on the same line in both directions.
- A click in the tree moves to that file's header, and the tree marks the file under the cursor.
- A comment made in all-files mode lands on the right file and line.
- An agent's change to one file keeps the cursor and scroll where they were, in that file and in
  the files around it.
- `[editor] all-files = false` opens one file, as today.
- Perf, in `src/ui/tests/perf.rs`: opening the 150-file review in all-files mode stays within
  today's 5-second budget, and the key run gets phases that scroll the whole review with
  `C-d`, search it, and press `]g` through it. A test checks that no more than about 50 file
  documents are kept at once.

## Decisions

- **Stacked documents rather than one document.** One `Rope` for the whole review would let every
  Helix command cross files for free. It was rejected for three reasons. It has to be rebuilt
  whenever any file changes. A selection across two files can't be commented on. And it holds the
  text of every file at once, so it can't be virtualized.
- **A mode rather than a fourth view.** Each of the three views is useful across all files, so
  the mode and the view are chosen separately.

## Open

- Should `space d` and `space D` in all-files mode both list every thread in the review, since
  the whole review is on screen?
- When the agent adds or removes a changed file, where should the cursor go if it was in that
  file?

## Relevant code

- `src/ui/mod.rs`: `State`, `App` (`alt`, `folds`, `view_kind`), `Alt`, `load`, `ensure_alt`,
  `tree_entries`, `files_changed`, `render`, and next and previous file in `execute`.
- `src/ui/diffview.rs` `rows`, `build` and `Folds`.
- `src/ui/view.rs` `File` and `draw`.
- `src/ui/folding.rs`.
- `src/ui/tree.rs`, for marking the file under the cursor.
- `src/ui/keymap.rs` `normal_defs`, for `' v a`.
- `src/config.rs` `Editor`.
- `src/ui/tests/perf.rs`.
