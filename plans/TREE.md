# File tree: collapsible folders

The tree lists every changed file under its folders, and every folder is always open. In a big
review that's a long list. Folders should open and close, the way they do in Helix's file
explorer, yazi and GUI editors.

This builds on [MOUSE.md](MOUSE.md), which changes how the tree takes clicks and lets its view
scroll apart from its cursor. Do it after that.

## Where we are

`src/ui/tree.rs` has three types:

- **`Entry`:** a changed file, with its path, lines added and removed, and open threads.
- **`Row`:** what's drawn. `name` is already indented; `path` is `Some` for a file and `None` for
  a folder. A folder row doesn't record its own path.
- **`Tree`:** the rows, a cursor and `focused`.

`j`/`k` (`Tree::step`) skip folder rows, so the cursor is always on a file. `App` builds the tree
from `App::tree_entries`, and rebuilds it when files change on disk (`files_changed`). A rebuild
keeps `focused` and the selected file.

The palette's tree keys are in `App::context_keys`. Its description for `space e`,
`file_explorer`, is "Open the file tree", although the key also focuses and closes it.

## Design

- **Rows know what they are.** `Row` gets `depth`, a folder's own path (`src/ui/`), and a `kind`
  of file or folder. `name` stops being indented; `render` indents by `depth`.
- **Closed folders are a set of paths** on `Tree`, `collapsed: HashSet<String>`. The rows drawn,
  and moved through, are the ones not inside a closed folder: the *visible rows*.
- **A folder row shows** `▾` when open and `▸` when closed. It also shows the totals of the files
  under it (`+added -removed ●threads`), so a closed folder still tells you what's in it.
- **The cursor lands on folders too.** `j`/`k` move through every visible row, because keys need
  to be able to reach a folder to open or close it. This is a change from today, where they skip
  folders. `ret` and `l` on a file still open it.
- **Keys:**

  | Key | On a folder | On a file |
  |---|---|---|
  | `ret` | Open or close it | Open the file (as now) |
  | `l`, right | Open it; if already open, go to its first child | Open the file (as now) |
  | `h`, left | Close it; if already closed, go to its parent | Go to its folder |

  The `h`/`l` rules are those of Helix's file explorer and yazi.
- **Clicks:** a click on a folder opens or closes it. Following MOUSE.md, it also focuses the
  tree.
- **The current file is always visible.** When the file on screen changes (`g n`, a picker, a
  thread, the jumplist), the folders holding it open. If the tree isn't focused, its cursor moves
  to the file, as `select` does today.
- **A rebuild keeps closed folders closed.** `files_changed` passes `collapsed` on to the new
  tree along with `focused` and the selection. A closed folder that no longer exists is dropped.
- **Search jumps to a match**, the way `/` works in the code:
  - `/` (or `?` to search backwards) opens the command line as `search:`, through a new
    `PromptKind::TreeSearch { forward, origin }`, where `origin` is the tree's cursor.
  - As you type, the cursor jumps to the next row whose name, as the panel shows it, matches:
    `view.rs` for a file, `ui/` for a folder. The paths above it don't count, so `src` finds the
    `src/` folder and not every file under it. Names are matched with the same regex and smart
    case as the code's search (`commands::search_regex`).
  - Rows inside closed folders are searched too, in tree order. A match inside a closed folder
    opens that folder, and the folders above it, so the match is on screen.
  - `ret` keeps the match, `esc` puts the cursor back, and `n`/`N` go to the next or previous
    match, wrapping at the ends.
  - The pattern goes in the same `/` register as the code's search (`last_search`), as Helix has
    one search register. So `n` in the tree repeats a search made in the code, and the other way
    round.
  - No match: "No matches", and the cursor stays where it was.
- **The palette:**
  - The tree's keys in `C-g` gain `h`, `l` and `ret` on a folder, and `/`, `?`, `n` and `N`.
  - `file_explorer`'s description becomes "Open, focus or close the file tree", so searching
    "close" finds it.
  - `plans/KEYMAP.md` lists the new keys.

## Steps

One PR on a `tree-folders` branch, after the mouse PR. Each commit passes `cargo fmt`, `clippy`
and `cargo test`.

1. **Rows know what they are:** add `depth`, the folder path and `kind` to `Row`, and render the
   indent from `depth`. The tree should look the same.
2. **Folders open and close:** `collapsed`, the visible rows, `j`/`k`/`h`/`l`/`ret`, folder
   totals, and the `▸`/`▾` markers.
3. **Clicks on folders, revealing the current file, and keeping closed folders** across a
   rebuild.
4. **Search:** `/`, `?`, `n` and `N` in the tree, through `PromptKind::TreeSearch`.
5. **The palette and docs:** the tree's keys in `C-g`, the `file_explorer` description and
   `KEYMAP.md`.

## Tests

- **Unit tests in `tree.rs`:**
  - A closed folder hides everything under it, including nested folders, and shows their totals.
  - `j`/`k` move over visible rows only.
  - `h`/`l` follow the table above, at the top, middle and bottom of the tree.
  - Clicking a folder toggles it.
- **App tests:**
  - Opening a file inside a closed folder, with `g n` or a picker, opens the folder and selects
    the file.
  - A rebuild after a file changes on disk keeps folders closed.
  - The `C-g` palette, with the tree focused, lists the folder and search keys.
  - Search in the tree:
    - Typing `/` and a pattern jumps the cursor to each match as it grows.
    - Only the shown name matches: `src` finds the `src/` folder and not the files under it.
    - A match inside closed folders, nested ones included, opens them, and folders without a
      match stay closed.
    - `esc` puts the cursor back, and `ret` then `n`/`N` step through the matches, wrapping at
      the ends.
    - `?` searches backwards, and a pattern with no match leaves the cursor put.
    - A search made in the code repeats in the tree with `n`.
- **A screen snapshot** of a tree with a closed folder and a nested open one.

## Out of scope

- Remembering closed folders across glos sessions.
- Opening or closing every folder at once.
