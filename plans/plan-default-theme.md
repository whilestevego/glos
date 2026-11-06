# catppuccin_mocha as the default theme

**Status:** Ready to build (2026-11-06). This answers the overview's open question on the default
theme, in `completed/plan-overview.md`.

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
