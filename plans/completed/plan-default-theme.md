# catppuccin_mocha as the default theme

**Status:** Implemented (2026-11-06) in PR #10. This answers the overview's open question on the
default theme, in `completed/plan-overview.md`.

When `config.toml` doesn't set a theme, glos uses Helix's `default`. It has low contrast, and
collapsed thread markers, drawn in its `info` colour, are hard to read in it. glos's default
becomes `catppuccin_mocha`, which is one of Helix's built-in themes. Setting `theme = "default"`
still gives Helix's default.

## Steps

1. Change the default in `src/config.rs` `Config::default`, and the comment in the README's
   "Settings" section.
2. Check every part of the screen in catppuccin_mocha, in a terminal with truecolor and in one
   with 256 colours: thread rows collapsed and open, the comment box, the gutter marks, fold
   bands, the changed-line and changed-word tints, the tree, pickers, the key help panel and the
   dashboard. Fix any that are hard to read.
3. Check `src/doctor.rs`, which loads `default` as a fallback, still makes sense.

## Tests

- Tests that check colours read them from the theme in use, not from fixed values, so they pass
  with either theme. Any that hard-code `default`'s colours are changed to read the theme.
- A test that `Config::default().theme` is `catppuccin_mocha` and that it loads.

## Relevant code

- `src/config.rs` `Config::default`.
- `src/theme.rs`.
- `src/doctor.rs`.
- `README.md`, "Settings".

## As built

The default lives in one constant, `DEFAULT_THEME` in `src/config.rs`. `Config::default`, the
fallback in `src/ui/mod.rs` `run` when your theme won't load, and the same fallback in
`src/doctor.rs` all use it. The screen tests in `src/ui/tests.rs` now load the theme from their
config, so they run on the default theme. Every test passed unchanged, which shows the colour
tests already read their colours from the theme.

Checking the screen found one real problem, and it wasn't specific to catppuccin_mocha. In
256 colours, the changed-line tints of every dark theme came out grey. Added and removed lines got
the same grey, which was also the current line's colour, and changed words got another shared
grey. `tint_256` in `src/theme.rs` mapped the mix straight to the nearest of the 256, and a
slight tint over a dark background is nearest a grey. It now keeps the mix's lightness, takes the
hue of `diff.plus` or `diff.minus` with a minimum of colour, and picks the nearest non-grey entry
of the colour cube. It keeps mixing further while the result matches the background, or for a
word tint, its line's tint. catppuccin_mocha now gets dark green and dark red lines in 256
colours, and its text has a contrast of at least 4.8 on every tint. A test checks ten dark and
light themes for coloured tints that tell the two sides apart. Truecolor didn't change.

Everything else read well in catppuccin_mocha in both colour modes. Unselected line numbers are
the faintest, as they are in Helix with the same theme.
