# All files on one screen

**Status:** In progress on branch `all-files-2`. Steps 1 and 2 were merged on 2026-11-08 in
PR #12. Steps 3 to 5 are built on this branch. The mode is off by default until step 6, the
last pull request.

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
- **Filters and scopes apply.** `:filter` and `' o` decide which files are in the scroll.

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

## Decided

- **The thread pickers keep their meanings (2026-11-06).** `' d` lists the threads in the file
  under the cursor, and `' D` lists the whole review's, as in one-file mode. The keys stay
  different, and they match Helix's diagnostic pickers.
- **A file that drops out of the review (2026-11-06).** If the agent's change means the file
  under the cursor is no longer changed, it leaves the scroll, and the cursor goes to the header
  of the file that took its place, or the last file. The status line says which file went. A new
  file appears in its place in the order, and the cursor stays where it was.

## As built so far

Steps 1 and 2, in the first pull request:

- **Step 1 wasn't needed.** The scroll is kept as a place, a file and a line of its document
  (`src/ui/all.rs` `Pos`). Only the files between the top of the screen and the cursor are
  measured, so nothing needs every file's row count. glos has no scrollbar, and `G` goes to the
  last file directly. Opening 150 files of 2,000 lines in all-files mode takes about 0.3 seconds.
- **The cursor's file is drawn as in one-file mode.** `App::draw_all` draws the file under the
  cursor from its own view, so folds, threads and every key work on it unchanged. The other
  files are drawn from views built by `built_for`, which `built` now calls for the file on
  screen. Moving past a file's first or last line moves the cursor into the next file
  (`all_files_motion`).
- **The header has the path, `+added -removed` and the open threads.** The kind of change
  stays in the notices under the header, as in one-file mode, rather than repeating there.
- **Headers are rules, with a blank row above them.** The first build drew each header as a
  bar in `ui.statusline.inactive`, which the reviewer found too faint to tell where one file
  ended. Now a blank row comes before each file after the first, and the header is a rule
  across the screen in bold `ui.linenr.selected`, such as `── ▼ b.rs  +1 -0 ─────`
  (`draw_header`). While a file's header has scrolled off the top, a copy of it stays pinned
  on the top row, as GitHub's file headers do (`pinned`).
- **The empty line after a file's last newline isn't shown** in all-files mode, so files follow
  one another without a blank numbered line between them.
- **Collapsed files.** A collapsed file is its header, its notices and a summary line such as
  `↕ 42 lines` (`summary`), and its header shows `▶` rather than `▼`. The summary
  doesn't say how to open the file, because the reviewer found that repetitive on every
  collapsed file, and a generated file's "enter expands" notice gives way to it. A binary file
  is its header and notices. The cursor stops on either as one line. A click on any header,
  pinned or not, or on a summary, collapses or expands its file, and `enter` does the same for the cursor's
  file when it isn't on a fold or a thread (`toggle_file`). The file's folds are kept by path,
  so they're as they were when it opens again, and so is the cursor. A file the reviewer
  collapsed or expanded stays loaded, so it doesn't go back to its default when it's dropped.
- **At most 50 documents** are kept (`MAX_LOADED`). Files far from the screen are dropped, and
  read again when they come back.
- **The mouse came forward from step 5.** In one-file mode the wheel moves the cursor within its
  file, which stopped scrolling at the end of a file. `all_wheel` scrolls the review and moves
  the cursor only to keep it on screen. A click moves the cursor into the file under it
  (`all_focus_at`), and a drag stays in the file it started in.
- **Not yet.** Search, `]g` and the thread keys still work within the file under the cursor.
  They're step 4.

Step 4, in the second pull request:

- **One helper crosses files for every jump.** `App::jump_across` runs a jump in the cursor's
  file, and when it finds nothing more there it tries each later file (earlier, going back)
  from its start (end). Search uses it with wrapping round the review, and `]g`, `[g`, `]r` and
  `[r` without. `]R` and `[R` go to the last and first thread of the review.
- **Search checks a file's text before opening it.** Opening every file of the 150-file review
  to search it took 9 seconds. `may_match` runs the regex over the file's text first, so a
  search that matches nothing takes about 30 ms. A search as it's typed starts again from the
  file it began in, and `esc` goes back there (`PromptKind::Search` `all`).
- **The tree already followed the cursor.** It marks the file on screen when it draws, so it
  needed no change, only a test.
- **`gn` and `gp`** already went to a file's header in step 2.

Steps 3 and 5, in the same pull request:

- **Every view builds each file's view.** `built_for` now takes the file to build, so
  `AllFiles::prepare` builds the unified or side-by-side view of each file near the screen, as
  it builds the folded whole-file view. The cached views are built again when the view
  changes. Switching views moves the top of the screen to the start of its file, because line
  numbers differ between views, and the screen then follows the cursor.
- **Side by side keeps two columns for every file.** `layout` splits the screen whenever
  all-files mode is in the side-by-side view, even when the cursor's file is binary or
  collapsed. Headers go across both columns, and each file's old side is drawn in the left
  column with blank thread rows, as in one-file mode (`blank_threads`).
- **Notices come from the file itself.** The built views have no notices, so "new file" and the
  rest would have gone missing from the headers in the unified and side-by-side views.
- **Step 5 was mostly done in step 2.** Clicks on the left column of side by side move the
  cursor into that file. They don't place the cursor on a line of the old side, as one-file mode
  doesn't either.

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
