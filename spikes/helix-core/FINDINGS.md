# M0 spike 4: helix-core as a dependency — WORKS

`helix-core = { git = "https://github.com/helix-editor/helix", tag = "25.07.1" }` builds on
rustc 1.94 (Helix MSRV 1.82) and drives motions and selections over a ropey `Rope`.
`cargo test`: 6 tests in Helix notation (`#[h|]#ello`) pass: `w`/`b`/`e` with counts, select-mode
`w`, `x` (once, twice, `2x`), `%`, `mm`, `miw`, `mi(`, `ma(`, `mi2(`, two cursors.

## Weight
- Pulls in helix-core, helix-loader, helix-stdx, helix-parsec only. No helix-view/term/event/lsp.
- ~150 crates total (normal + build). Notable: tree-house 0.3.0 + tree-house-bindings 0.2.4
  (vendored tree-sitter C, built with cc), nucleo 0.5, imara-diff 0.2, regex, globset, url, toml
  0.8, chrono, encoding_rs, libloading, etcetera, which, icu_* (via url).
- Clean release build: 24 s wall (102 s CPU) on this Mac.
- Stripped release binary: 537 KB vs 337 KB empty, so +200 KB. Unused parts (syntax, loader)
  are dead-stripped; this will grow once glos uses `syntax`.
- helix-loader's build.rs only runs `git rev-parse` for a version string. It does not fetch or
  build grammars. No HELIX_RUNTIME or other env var needed for motions/selections; the runtime
  dir is only touched by `syntax::Loader` (dlopens grammars from runtime/grammars).

## Gotchas
- **Lockfile.** A git dependency doesn't bring its Cargo.lock. Fresh resolution picked
  kstring 2.0.5 (needs rustc 1.96) and failed. Fix used here: seed Cargo.lock from Helix's own
  lock at the tag (`cp .../Cargo.lock`), then build. In M1, commit glos's lock and seed it the
  same way; recheck when bumping Helix.
- **Cursor invariant.** helix-view's `Document::set_selection` calls
  `Selection::ensure_invariants`, which keeps normal-mode cursors one grapheme wide. Without it,
  `mm` gives a 0-width cursor. glos must call it after every command.
- **tree-sitter overlap (for spike 2).** helix-core already links tree-house 0.3.0 and its
  vendored tree-sitter C runtime. Highlighting should use that same tree-house version rather than
  the `tree-sitter` crate or tree-house 0.4, or there are two tree-sitter C runtimes in one binary.
  In tree-house-bindings 0.2.4, `Grammar` is created only by `Grammar::new(name, path)` (dlopen);
  the constructor from a static `tree_sitter_x()` pointer is private. Compiling grammars in needs
  a workaround: transmute (Grammar is `repr(transparent)` over NonNull), an upstream PR, or
  dlopen from a temp dir. Spike 2 should decide.

## Wrappers glos must write
helix-core has the pure functions. Everything keymap-shaped lives in
helix-term/src/commands.rs (6825 lines): counts, modes, select-mode `extend_*` variants,
on-next-key (`f`/`t`/`m`), and the editor/document plumbing. Wrappers are thin. In this spike,
`move_word_impl`/`extend_word_impl` are 3–5 lines each, `extend_line_impl` (`x`) ~10, `%` 1,
`match_brackets` ~8, `select_textobject` ~20 for w/W/p/(/m, plus tree-sitter objects later.
`mig` (textobject_change) is a hunk text object and fits glos directly.
Needed for M1, roughly: hjkl (move_horizontally, move_vertically; the `_visual` variant only
with soft wrap), w/b/e/W/B/E, gg/ge/gh/gl/gs, x/X, %, ;, alt-;, `,`, f/t/F/T, `/` n N (regex
over the rope via regex-cursor), mm/mi/ma, s/S/C, ctrl-d/u/f/b (view scrolling, glos's own), v
select mode, counts. Estimate 600–1000 lines taken from commands.rs. Those are MPL-2.0: keep
them in one file (e.g. `src/ui/commands.rs`) with the MPL notice.

## Recommendation for M1
Keep the git dependency, pinned to the tag, with the lock seeded from Helix. No need to copy
helix-core in. Port the command wrappers from helix-term into one MPL-2.0 file, and keep this
notation-based test style (`run(input, keys) -> output`) for the keymap tests.
