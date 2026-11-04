# Spike 2: highlighting — findings

**Works.** Grammars compiled into the binary at Helix 25.07.1's pinned revisions, with Helix's
queries and themes embedded via `include_str!`, highlight Rust, TypeScript, Python, YAML, Ruby and
Markdown (with markdown.inline and a fenced rust block injected) through `tree-house`.
Run: `scripts/fetch.py [langs…]`, then `cargo run --release -- <file> [theme]`; `cargo test`
checks query inherits and theme inherits.

## tree-house (use it; no need for tree-sitter-highlight)
- Helix 25.07.1 locks `tree-house 0.3.0`; the same version as helix-core, so one copy of the
  tree-sitter C library. Bindings resolved to 0.2.4 (Helix locks 0.2.1); pin `=0.2.1` in M1 if
  helix-core is a dependency so Cargo.lock matches Helix.
- Static grammars: enable `tree-house-bindings`' `tree-sitter-language` feature, then
  `Grammar::try_from(LanguageFn::from_raw(tree_sitter_rust))`. No dlopen, no hack.
- API: implement `LanguageLoader` (`language_for_marker`, `get_config`), build
  `LanguageConfig::new(grammar, highlights, injections, locals)`, call `config.configure(...)`
  with Helix's longest-prefix scope match (copied from helix-core `syntax.rs`), then
  `Syntax::new` + `highlighter::Highlighter::new(&syntax, rope, &loader, range)`. Loop on
  `next_event_offset()`/`advance()`: `Refresh` restarts from the base style, `Push` patches it.
  The `range` argument gives "only highlight visible lines" for free.
- `default-features = false` drops its test fixtures.

## Inherits
- Queries: `tree_house::read_query(lang, |l| text_for(l))` resolves `; inherits: ecma,_typescript`
  itself (Helix calls exactly this). fetch.py follows the same directive to vendor parents
  (typescript → ecma, _typescript).
- Themes: ~40 lines re-implement helix-view's `load_theme`/`merge_themes`/`merge_toml_values`
  (`inherits` recursion; palette merged one level deeper). Test: catppuccin_frappe (inherits
  mocha) gives frappe's mauve for `keyword`. `"default"` = Helix's root `theme.toml`.

## Sizes (macOS arm64, release)
- Rust + TS: 5.6 MiB binary (5.0 MiB stripped), includes toml/ropey/std.
- All 20 plan grammars (incl. tsx, markdown_inline): 18.3 MiB (17.6 MiB stripped) →
  grammars cost ~13 MB. Static libs: cpp 3.4M, sql 2.0, ruby 2.0, bash 1.3, ts/tsx 1.1 each,
  rust 1.0, rest ≤0.5. Dropping cpp+sql+ruby saves ~7 MB if size matters.
- Vendored grammar source: 92 MB raw (generated parser.c), 4.8 MB gzipped. Helix files: 408 KB.
  Spike gitignores vendor/; see M1 recommendation.

## Gotchas
- Scanners `#include "../../common/scanner.h"` (typescript, markdown): vendor keeping repo layout.
- ruby and yaml have C++ scanners (`scanner.cc`): build.rs compiles them with `cpp(true)`.
- Every languages.toml entry sharing a compiled grammar loads too (jsx, json-ld, quarto…);
  restrict to the plan's list.
- **Query compilation is slow:** cpp 57 ms, ruby 41, rust 30, tsx 27, ts 25, bash 23 (release).
  Eager loading of all 20 is ~260 ms at startup. Compile lazily per language (Helix uses a
  OnceCell), and off the UI thread, to keep the 100 ms first screen.
- Parse + highlight of a 700-line Rust file: ~6 ms.
- Theme colours: only hex handled here; M1 needs named ANSI colours and 256 indexes (Helix's
  `ThemePalette` defaults), plus `bg`, underline styles and the remaining modifiers.
- Injections by `Name` and exact `Match` only; M1 needs Helix's injection-regex, filename and
  shebang matching.

## Recommendation for M1
- Use tree-house 0.3.0 with the `tree-sitter-language` feature; port build.rs and fetch.py as-is.
- Vendor only `src/` (+`common/`) as here. Committing it adds ~5 MB to the git history per Helix
  upgrade; acceptable, or keep vendor/ out of git and run fetch.py in CI/`build.rs` (needs network
  at build time — worse for crates.io publishing). Leaning: commit it.
- Lazy per-language query compilation from day one.

## After the spike

`helix/`, `vendor/`, `build.rs` and `scripts/fetch.py` moved to the repo root for M1 (this crate no
longer builds on its own). Correction: Helix's lock pins tree-house-bindings 0.2.1, which has no
`tree-sitter-language` feature; glos needs 0.2.4, still one copy of tree-sitter under tree-house
0.3.0.
