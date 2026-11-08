# Language servers

**Status:** Draft (2026-11-06).

While you review, you often want to know what a name refers to, where else it's used, or whether
the change compiles. Helix answers those questions with language servers, and glos can do the
same with the same keys. This plan adds diagnostics, going to definitions and references, hover,
and the symbol pickers. Rename and code actions change files, so they wait for
`plan-in-place-editing.md`.

## What it adds

| Keys | Command | Notes |
|---|---|---|
| `space d` `space D` | Diagnostics in this file, and in the whole workspace | Today these pick threads |
| `]d` `[d` `]D` `[D` | Next, previous, last and first diagnostic | Today these move between threads |
| `gd` `gD` `gy` `gi` | Go to the definition, declaration, type definition or implementation | |
| `gr` | Go to references, in a picker | |
| `space k` | Hover: the type and docs of what's under the cursor | |
| `space s` `space S` | Symbols in this file, and in the workspace | Today `space s` submits |
| `space h` | Select the references to the symbol under the cursor | |
| `space a` `space r` | Code actions and rename | After `plan-in-place-editing.md` |

Diagnostics show the way Helix shows them by default: a mark in the gutter, the text underlined
in the theme's `diagnostic.error`, `diagnostic.warning`, `diagnostic.info` and `diagnostic.hint`
styles, and the message in a popup when the cursor is on it. glos reads Helix's
`[editor] end-of-line-diagnostics` and `[editor.inline-diagnostics]` settings, with Helix's
defaults.

Helix wins key clashes (see `DECISIONS.md`). Before this plan's keys are bound, step 2 of
`plan-review-menu.md` moves submit, the thread pickers and the thread bracket keys out of the way.

Going to a definition in a file outside the review opens it for context, as `space f` does. It's
`Cur::Context` in `src/ui/mod.rs`, and it goes on the jumplist.

## How it works

- **The client is Helix's.** `helix-lsp` from Helix 25.07.1 has the client, the server registry,
  and the conversions between LSP positions and `Rope` positions. Its dependencies are
  `helix-core`, `helix-loader` and `helix-stdx`, which glos already builds, plus more `tokio`
  features. It goes into `crates/glos-helix` like the rest of Helix, so glos's own code still
  never names a Helix crate. The first step is a spike that proves the registry starts
  rust-analyzer from glos's `Loader` and returns diagnostics. If `helix-lsp` turns out to need
  `helix-view`, the fallback is a small client of glos's own on `helix-lsp-types`.
- **Which servers run.** Helix's `languages.toml` is already embedded for highlighting
  (`crates/glos-helix/helix/languages.toml`). Its `[language-server]` table and each language's
  `language-servers` list say which server to start. You can add to them or override them in
  `~/.config/glos/languages.toml`, in Helix's format. glos never reads Helix's own config
  (see "Helix fidelity" in `completed/plan-overview.md`).
- **When they run.** A server starts the first time a file of its language is shown, and stops
  when glos quits. A server that isn't installed is skipped quietly, and the status line says so
  once. `glos doctor` lists each language's server and whether it's on your `PATH`, as
  `hx --health` does.
- **The workspace.** For a git review, it's the worktree. For `glos diff old new` it's the `new`
  folder, or the folder of the `new` file. Helix's root markers in `languages.toml`, such as
  `Cargo.toml`, narrow it the way Helix does.
- **What the server sees.** Servers read the files on disk. glos opens a file with the server
  (`didOpen`) using the text on disk, and tells it when the text changes, which today happens
  when glos reads a file again in `files_changed`. Once `plan-in-place-editing.md` is built,
  every edit is sent too, using `helix-lsp`'s conversion of a `Transaction` into LSP changes.
- **Only the new side.** Servers know nothing about the old side. In the unified and side-by-side
  views, diagnostics and the `g` keys work on lines from the new side, and do nothing on removed
  lines.
- **When the new side isn't on disk.** For `glos diff A B` and the per-commit scope, the files on
  screen are a commit, not the files on disk, so servers stay off and the keys say why in the
  status line.
- **Waiting for answers.** The event loop in `src/ui/mod.rs` `event_loop` waits up to 500 ms for a
  key. Server answers arrive on a `tokio` runtime thread, and the loop has to show them as soon as
  they come. While a request is outstanding, the loop polls every 20 ms instead, or a thread wakes
  it.

## Steps

1. **Spike.** Start rust-analyzer through `helix-lsp` from `glos-helix`, open one file, and print
   its diagnostics. Decide between `helix-lsp` and a client of glos's own.
2. **Keys.** Build step 2 of `plan-review-menu.md`.
3. **Servers and diagnostics.** Start and stop servers, sync text, and show diagnostics in the
   gutter, underlined and in the popup. Add `]d` and its family, `space d` and `space D`.
4. **Navigation.** `gd`, `gD`, `gy`, `gi`, `gr`, `space h` and hover on `space k`.
5. **Symbols.** `space s` and `space S`, using the existing picker in `src/ui/picker.rs`.
6. **`glos doctor`**, and the README.
7. **After editing:** rename on `space r`, code actions on `space a`, and completion and signature
   help in insert mode.

## Tests

- A fake server in the tests, a small binary in the test crate that speaks LSP over stdio and
  answers from fixed data, so the tests don't need rust-analyzer. It checks the requests glos
  sends as well.
- Diagnostics appear in the gutter and in the popup, and `]d` visits them in order.
- `gd` to a file outside the review opens it for context, and `C-o` comes back.
- Symbol pickers list the fake server's symbols and jump to them.
- Servers stay off when the new side is a commit, and the keys say why.
- A missing server doesn't stop glos, and `glos doctor` reports it.
- The perf run in `src/ui/tests/perf.rs` stays inside its budget with the fake server running.

## Open

- Should diagnostics be part of the review? For example, `' s` could warn before you
  approve a change with errors in it, or the agent could get the diagnostics with your review.
- In the all-files mode of `completed/plan-all-files.md`, should `space d` list every changed file's
  diagnostics rather than one file's?

## Relevant code

- `crates/glos-helix/Cargo.toml`, `src/lib.rs` and `src/syntax.rs` (the `Loader` and
  `languages.toml`).
- `src/ui/mod.rs` `event_loop`, `files_changed`, `Cur` and the jumplist.
- `src/ui/view.rs` `draw`, for the gutter, underlines and the popup.
- `src/ui/keymap.rs` `normal_defs`.
- `src/ui/picker.rs`.
- `src/config.rs`, for `[editor]` settings and `languages.toml`.
- `src/doctor.rs`.
