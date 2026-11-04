# Folding

Status: built on the `folding` branch (2026-11-04). Decisions are in `DECISIONS.md`.

Hide unchanged code so a review shows the changes, as GitHub's pull request view does, plus folds
you make by hand and Helix's tree-sitter text objects to make them with.

## Automatic folds

- **Every view folds:** unified, side by side and the whole-file view. A stretch of unchanged lines
  becomes one fold line: `⋯ 40 unchanged lines`.
- **Context:** `context` lines are kept before and after each change. A stretch is only folded if
  that hides at least 2 lines; folding one line into one fold line saves nothing.
- **Threads are never folded:** a line with a thread (yours or an agent's note) counts as a change,
  so it keeps its context and `]d`, `]u` and the pickers never land in a fold.
- **Starting state:** every view starts folded, the whole-file view too, so a new review opens on
  its changes. `zR` shows the whole file. Files you open for context (`space f`) have no changes,
  so they're shown whole.
- **Reset on rebuild:** when the agent's changes reload a file, its automatic folds are rebuilt from
  the context setting, and folds you'd opened close again. Switching files and coming back keeps
  them as you left them, for this session.

## Keys

All in the `z` view menu (and sticky `Z`), Vim's letters where Helix leaves them free (`zc` is
Helix's "center", so closing a single fold is `za`).

| Key | Does |
|---|---|
| `zo` | Open the fold at the cursor |
| `za` | Toggle the fold at the cursor (closes an opened one again) |
| `zR` | Open every fold in the file |
| `zM` | Fold everything again, by the context setting |
| `z+` / `z-` | One more / one fewer context line, for this session (minimum 0) |
| `zf` | Fold the selected lines, one fold per selection |
| `zd` | Remove the hand-made fold at the cursor |
| `ret` | On a fold line: open it (it already opens threads) |
| click | On a fold line: open it |

`Ctrl-G` and the key help panels list them like any other keys.

## Hand-made folds

- **`zf` folds any selection:** lines (`x`), a paragraph (`mip`), a function (`maf`), a class
  (`mac`) and so on. Select first, then fold, as Helix does everywhere.
- **They can hide changes and threads,** since you asked for them. The fold line says what's inside:
  `⋯ 40 lines · 3 changed · 1 thread`. `]g`, `]d`, `]u`, `gn` and the pickers open a fold when
  they land inside one.
- **Reset on rebuild,** like automatic folds: new changes from the agent clear them.

## Tree-sitter text objects

Helix's, with its own queries (`textobjects.scm`, vendored by `cargo xtask fetch-helix`, like the
highlight queries).

- `m a` / `m i` + `f` function, `t` type (class), `a` argument, `c` comment, `T` test, `e` entry,
  `x` tag (HTML/XML).
- `]f` `[f`, `]t` `[t`, `]a` `[a`, `]c` `[c`, `]T` `[T`, `]e` `[e`, `]x` `[x`: next and previous.
  Check each against glos's own `]`/`[` keys (`]g`, `]d`, `]u`, `]C`) before binding it.
- They work in every view, on the new side's syntax tree. In unified and side by side, an object
  that crosses a fold or the other side's lines selects only the rows it covers on the new side.
- This closes part of M1's tree-sitter motions gap. `alt-o`/`alt-i` (expand and shrink selection)
  stay out of scope.

## Settings

```toml
[editor]
diff-context = 3   # unchanged lines kept around each change; 0 shows changes only
```

`z+` and `z-` change it for the session only. `glos doctor` reports the value.

## Not doing

- Expanding a step at a time (GitHub's ↑/↓ arrows): `zo` opens a whole fold.
- Keeping folds across reloads or restarts.
- Folding by syntax automatically (every function body): folding is driven by changes and by
  your selections.

## Tests

- The fold arithmetic: context 0, 1 and 3; changes at the very start or end of the file; adjacent
  changes whose context overlaps; a 1-line gap stays unfolded; a thread on an unchanged line.
- Each key on each view: `zo`, `za`, `zR`, `zM`, `z+`/`z-` (a fold you'd opened stays open),
  `zf` with one and several selections, `zd`, `ret` and click on a fold line.
- The cursor and the scroll position stay on the same line of code through every fold change.
- Comments taken on lines next to a fold, and selections across one, map to the right new-side
  lines.
- `]g`/`]d`/`]u` and the pickers opening a hand-made fold; the fold line's marker counts.
- A rebuild after new changes resets folds; switching files keeps them.
- Text objects: `maf`, `mif`, `mac`, `]f`/`[f` on Rust and one other language, in the file view
  and in unified, including a function that's partly removed.
- Config: `diff-context` parsing, its default, and an invalid value.
- Snapshot tests of each view with folds, and the updated `Ctrl-G` and `z` menu.
