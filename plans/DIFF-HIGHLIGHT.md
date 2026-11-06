# Diff highlighting: tinted lines and changed words

Today a change shows only as a coloured bar in the gutter. This plan adds two things:

- **Line tints.** A changed line's background is tinted toward the theme's addition or removal
  colour.
- **Changed words.** Within a changed line, the words that actually changed get a stronger tint,
  as on GitHub and in `delta`. That's what makes the tint useful in a review: it shows the one
  number that changed in a long line.

Decided:

- The tint is the theme's background, shifted slightly toward `diff.plus` or `diff.minus`.
- In the whole-file view, modified lines take the addition tint, since they're the new text.
- Changed-word highlighting is part of this work.

## Where we are

- **The gutter:** `view::draw` (`src/ui/view.rs`) marks lines with `diff.plus.gutter`,
  `diff.minus.gutter` and `diff.delta.gutter`. Nothing else shows a change.
- **The whole-file view** shows the new file. `File.hunks` give the added and modified lines;
  removed lines appear only as a gutter mark.
- **The unified and split views** (`src/ui/diffview.rs`) build a document of `Row`s, each
  `Same`, `Added(n)`, `Removed(n)`, `Filler` or `Fold`, with syntax colours precomputed in
  `File.spans`.
- **The theme** (`src/theme.rs`) loads Helix themes. It maps RGB colours to the nearest of 256
  for `ColorSupport::Indexed`, and drops colour for `NO_COLOR`. Helix themes have no key for a
  diff line's background.

## Design

### The colours

Four styles, worked out once when the theme loads, from the theme's RGB colours and *before*
mapping to 256 colours:

| Key | Default | Used for |
|---|---|---|
| `diff.plus.line` | `ui.background` mixed 18% toward `diff.plus`'s colour | Added (and modified) lines |
| `diff.minus.line` | `ui.background` mixed 18% toward `diff.minus`'s colour | Removed lines |
| `diff.plus.word` | 35% toward `diff.plus` | Changed words in an added line |
| `diff.minus.word` | 35% toward `diff.minus` | Changed words in a removed line |

- **How to mix:** in OKLab, a perceptual colour space, so 18% looks equally subtle on a dark
  theme and a light one. Mixing RGB directly tends to muddy colours, especially mid-tones. The
  conversion is about 30 lines with no new crate.
- **Theme keys win.** If the theme sets one of these keys, glos uses it as is, so a theme can
  pick its own colours. These are glos's own keys; Helix ignores them. They go in the README's
  theme section.
- **When the mix can't be done:**
  - **No RGB background:** some themes leave `ui.background` unset to keep the terminal's own,
    and that colour can't be known without asking the terminal. These get no tint, unless the
    theme sets the keys. They keep the gutter marks.
  - **256 colours:** mix in RGB, then take the nearest of the 256. Small tints can round to the
    background itself. If a mixed colour comes out equal to the background, use the next
    nearest so the tint stays visible.
  - **`NO_COLOR`:** no tints, as now.
- **The 18% and 35% amounts** are a starting point, to be tuned by eye on the bundled themes:
  `default`, `catppuccin_mocha` and one light theme.

### Which lines

| View | `diff.plus.line` | `diff.minus.line` |
|---|---|---|
| Whole file | Added and modified lines (from `File.hunks`) | — (removed lines aren't shown) |
| Unified | `Row::Added` | `Row::Removed` |
| Split, new side | `Row::Added` | — |
| Split, old side | — | `Row::Removed` |

- **Width:** the tint fills the text area's width, past the end of the line, so a changed block
  reads as a band. Gutters, `Filler` rows, fold bands and thread rows don't get it.
- **Layering**, from back to front: the line tint, the changed-word tint, then the cursor line,
  selections and the cursor. Syntax colours stay on top, as foreground colours.

### Changed words

A new `src/diff_words.rs`, pure and with no UI, does the work:

1. **Pair lines.** Within each hunk, pair removed lines with added lines. When the counts are
   equal, pair them in order. Otherwise, pair greedily in order by similarity, and leave the
   rest unpaired. Unpaired lines get only the line tint.
2. **Split into words.** Tokenize each pair into runs of word characters, runs of whitespace,
   and single punctuation characters, so `foo(bar, 1)` is `foo`, `(`, `bar`, `,`, ` `, `1`, `)`.
3. **Diff the words.** Run `imara-diff` (already a dependency) over the two token lists.
   Tokens that differ become byte ranges on each side.
4. **Skip rewrites.** If more than half of either line changed, it was rewritten, not edited.
   Highlighting most of it adds noise, so it gets only the line tint.
5. **Limits:** don't pair lines in hunks over 500 lines, and skip lines over 2,000 bytes. The
   perf tests ([perf.rs](../src/ui/tests/perf.rs)) keep this honest.

**Where it's computed:** when a file's view is built, alongside `File.spans`, and cached on the
`File`. It's recomputed only when the file changes on disk, never on each draw.

**The whole-file view:** it shows only the new line, but highlighting the words that changed in
it, compared with its old pair, is still useful. You see `3` → `4` without opening the split
view.

## Steps

One PR on a `diff-highlight` branch. Each commit passes `cargo fmt`, `clippy` and `cargo test`.

1. **Colours:** OKLab mixing, the four keys with their defaults and theme overrides, and the
   256-colour and `NO_COLOR` cases. Unit tests only, so nothing on screen changes yet.
2. **Line tints** in all three views. Update the snapshots on purpose, and look at each one.
3. **Changed words:** `diff_words.rs` with its pairing, tokenizing, skip and limit rules, plus
   unit tests. Nothing is drawn yet.
4. **Drawing changed words** in all three views, cached on `File`. Update the snapshots on
   purpose.
5. **Docs:** the README's theme section, and `PLAN.md`'s "Viewing the code".

## Tests

- **Colour mixing:** OKLab round-trips, known mixes, and the mix staying between its two ends.
  The 256-colour mix never equals the background, and `NO_COLOR` gives none.
- **Theme keys:** a theme that sets `diff.plus.line` gets exactly that colour.
- **Line tints:**
  - Cells in an added line have `diff.plus.line`'s background, to the edge of the text area.
  - Gutter cells, filler rows and fold bands don't.
  - A selection's colour is drawn over the tint.
- **Changed words**, as unit tests:
  - Pairing equal and unequal hunks.
  - Tokenizing code and Unicode.
  - `let a = x + 1;` → `let a = x + 2;` highlights only `1`/`2`.
  - A rewritten line is skipped.
  - The size limits are respected.
- **Screens:** the whole-file, unified and split snapshots show both tints.
- **Performance:** the perf tests pass unchanged. Their fixtures change every 40th line, so
  they exercise word pairing at scale.

## Out of scope

- Detecting moved lines (`diff --color-moved`).
- Asking the terminal for its background colour (OSC 11) to tint themes that don't set one.

## As built

The work follows the plan, with these differences.

Steps 3 and 4 went in as one commit. On its own, `diff_words.rs` had no callers, so clippy
warned about dead code.

A changed range is trimmed of whitespace at both ends. Otherwise an inserted `b, ` would end in
a highlighted space, which shows nothing. A change that was only whitespace, such as
re-indenting, gets the line tint and no words.

Changes with only whitespace between them join into one range, so `two three` → `TWO THREE`
reads as one change and not two.

When a hunk's two sides differ in length, a removed line pairs with an added one only when at
least half their words are shared (`MIN_SIMILARITY`).

The line tint is 14%, not 18%. On a real terminal, 18% looked too strong. The word tint stays
at 35%. The tests check that the tints differ from the background and from each other, but not
how they look.

Each `File` keeps its changed words as byte ranges within each line (`File::words`). The
whole-file view works them out in `File::change`, and the unified and split views work them out
in `diffview::build`, alongside the hunks they already compute there.
