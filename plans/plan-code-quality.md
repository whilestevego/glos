# Code quality checks

**Status:** Built on `build/code-quality` (2026-11-22). The branch is not pushed, and it has no PR yet.

glos keeps its rules in `CLAUDE.md`, and nothing checks them. The workspace has no lint table and
no `rustfmt.toml`. It has no git hooks, no CI and no prose lint. This plan adds checks for the
code, the commit messages and the prose, and one command that runs all of them.

The model is `~/hobby/redouble`, which does most of this for Dart. Its Vale styles, its prose
script, its hooks and its `make ci` target are the source for each step here. The CI workflow
also copies `~/hobby/murmur/.github/workflows/ci.yml`: actions pinned to commit SHAs,
`concurrency` and `persist-credentials: false`.

## Where we are

- `Cargo.toml` has three crates: `glos`, `crates/glos-helix` and `xtask`. None has a `[lints]`
  table.
- A run of `cargo clippy` with `clippy::pedantic` and the restriction lints below gives 455
  warnings in 43 lints. 78 of them are `unwrap` or `expect` outside tests.
- glos has four `unsafe` sites:
  - `src/review.rs` `pid_alive` calls `libc::kill`.
  - `src/ui/mod.rs` calls `libc::raise(SIGTSTP)` to suspend glos.
  - `crates/glos-helix/src/syntax.rs` calls `LanguageFn::from_raw`.
  - `crates/glos-helix/build.rs` writes an `unsafe extern "C"` block for the grammars.
- `cargo xtask` has two tasks, `fetch-helix` and `keymap`, in `xtask/src/main.rs`.
- `cargo fmt --all --check` passes with the default settings.

## Design

### Lints

- `[workspace.lints]` in the root `Cargo.toml` sets the lints, and each crate has
  `[lints] workspace = true`.
- `clippy::pedantic` is `warn`. CI and the hooks run clippy with `-D warnings`.
- These restriction lints are `warn`: `unwrap_used`, `expect_used`, `dbg_macro`,
  `print_stdout`, `print_stderr`, `todo`, `unimplemented`, `undocumented_unsafe_blocks`,
  `allow_attributes` and `allow_attributes_without_reason`.
- `allow_attributes` makes each exception an `#[expect]`. An `#[expect]` warns when its lint stops
  firing, so a stale exception does not stay. `allow_attributes_without_reason` makes each
  exception give a `reason`.
- `clippy.toml` lets tests use `unwrap`, `expect`, `dbg!` and `print!`.
- `unsafe_code` is `deny`. Each `unsafe` site has an `#[expect(unsafe_code, reason = ...)]` and a
  `// SAFETY:` comment. `forbid` is not possible, because `forbid` cannot be overridden at a
  site.
- An exception is local. It goes on the function or the module that needs it, with its reason.
  A crate-wide exception is allowed only when the whole crate is the reason, such as `print!` in
  `xtask`.

### Formatting

- `rustfmt.toml` writes down the settings in use: `edition`, `style_edition`, `max_width`,
  `newline_style`, `use_field_init_shorthand` and `use_try_shorthand`. These are the defaults,
  or match the code now.
- It also sets `match_block_trailing_comma = true`. With the defaults alone, `cargo fmt` changes
  nothing.
- The reformat is its own commit, and `.git-blame-ignore-revs` lists it.

### Prose lint

- Vale checks Markdown files and the `//`, `///` and `//!` comments in Rust files. The styles
  are in `.vale/styles/GLOS/`:
  - `Banned.yml`: the banned words from `~/.claude/CLAUDE.md`.
  - `Dashes.yml`: no em or en dashes.
  - `SentenceLength.yml`: a sentence has 25 words or fewer.
  - `Structures.yml`: the banned structures that a regex can find, such as `not just`,
    `not only`, `it's not X, it's Y`, `think of it as` and `imagine`.
- Words that are banned only in one sense, such as "shift" as a noun, are not in the list. A
  regex cannot tell the senses apart.
- Vale reads Rust comments with its Rust grammar, which `vale sync` downloads.
- `cargo xtask prose [base]` reports only the alerts on lines that the branch adds since its
  merge base with `base` (default `main`). It skips `plans/completed/`, `plans/abandoned/`,
  `plans/KEYMAP.md`, the fixtures and the vendored Helix files.
- `cargo xtask prose-test` checks `.vale/fixtures/pass.{md,rs}` gives no alerts. It also checks
  that each rule fires on its expected line of `.vale/fixtures/fail.{md,rs}`.
- xtask downloads Vale 3.27.0, the version redouble pins, into `target/vale/`. It checks the
  SHA-256 of the download against a value in `xtask/src/vale.rs`. So each machine and CI use the
  same Vale, and nobody installs it by hand.

### Hooks and commands

- `.githooks/pre-commit` runs `cargo xtask pre-commit`: the format check and clippy.
- `.githooks/commit-msg` runs `cargo xtask commit-msg <file>`. It checks the subject, then runs
  Vale on the message.
- The subject rule: `type(scope): summary`, a type from `CLAUDE.md`, lowercase after the colon,
  no final period, and 72 characters or fewer. Subjects that git writes (`Merge`, `Revert`,
  `fixup!`, `squash!`) pass.
- `cargo xtask setup` sets `core.hooksPath` to `.githooks`.
- `cargo xtask ci` runs the format check, clippy, the tests, the prose lint and the prose
  self-test, in that order, and stops at the first failure.
- `.github/workflows/ci.yml` runs `cargo xtask ci`. `rust-toolchain.toml` pins the toolchain to
  1.94.0, the version in use. Dependabot updates the actions and the cargo dependencies.

## Steps

One branch, `build/code-quality`, with one commit for each step. Each commit builds and passes
`cargo test --workspace`.

1. This plan, its row in `README.md` and the decisions in `DECISIONS.md`.
2. `rust-toolchain.toml`.
3. `rustfmt.toml` and the reformat.
4. `.git-blame-ignore-revs`.
5. The pedantic fixes that `cargo clippy --fix` makes, and the ones done by hand.
6. `unwrap` and `expect` outside tests become errors that the caller handles, or an `#[expect]`
   with the invariant that makes them safe.
7. The `unsafe` sites get their `SAFETY` comments, and the lint table goes on.
8. The prose lint: `.vale.ini`, the styles, the fixtures, `xtask prose` and `xtask prose-test`.
9. The hooks and `xtask setup`, `xtask pre-commit` and `xtask commit-msg`.
10. `xtask ci`, the workflow and Dependabot.
11. `CLAUDE.md`, `README.md` and `plans/README.md` in STE, with an "Engineering standard"
    section in `CLAUDE.md`.

## Tests

- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- Unit tests in `xtask` for the subject rule. They cover each type, a scope, no scope, an
  uppercase summary and a final period. They also cover 72 and 73 characters, a missing colon
  and the subjects that git writes.
- Unit tests in `xtask` for the parser of added lines. They cover a hunk of one line, a hunk of
  many lines, a hunk that only removes, a new file and a deleted file.
- `cargo xtask prose-test` passes, and fails when a rule is removed from a style.
- `cargo xtask ci` passes on the branch.

## Out of scope

- Block comments (`/* */`). Vale's Rust grammar does not read them, and glos does not use them.
- A coverage floor.
- The plans in `plans/completed/` and `plans/abandoned/`. They are history and stay as written.

## As built

The work follows the design. These points depart from the plan or add to it.

- **Steps 5 to 7 became three lint commits.** An `#[expect]` warns when its lint is off, so each
  commit turns on its own lints with the fixes and the exceptions for them. The commits turn on
  `pedantic`, then the restriction lints, then `unsafe_code`.
- **Three lints are allowed for the whole workspace,** each with its reason in `Cargo.toml`:
  `format_collect`, `format_push_string` and `many_single_char_names`. The plan expected no
  workspace-wide exception. glos builds only short strings, and `write!` into a `String` needs
  an ignored `Result` at each of the 39 sites. Short names are the house style for pairs and
  maths.
- **Casts saturate.** `src/num.rs` has `cells`, `to_u32`, `to_isize`, `to_usize` and
  `line_number`, and `crates/glos-helix/src/lib.rs` has `to_u32` and `to_isize`. They replace
  about 80 `as` casts that could wrap.
- **Every other exception is local:** 55 `#[expect]` attributes, each with a reason, and one
  more that `build.rs` writes into the generated code. Most are
  `too_many_lines` (13) and `expect_used` on a stated invariant (10). The old `#[allow]`
  attributes, one for `deprecated`, one for `unnecessary_unwrap` and three for
  `too_many_arguments`, became `#[expect]` with a reason.
- **Small changes in behaviour.** `Agent::open_target` returns a message if a git source has no
  git data. It panicked before. The MCP server goes on after a tool call panics and poisons its
  mutex. `xtask keymap` returns an error for a Helix keymap that it cannot parse.
- **Vale reads `///` comments as plain text.** A code span in a doc comment is checked like the
  words around it. Vale does not read block comments.
- **`commit-msg` writes the message to `target/COMMIT_EDITMSG.md`,** because Vale picks its
  rules by the extension of the file.
- **The SHA in `.git-blame-ignore-revs` is the SHA on this branch.** A rebase or a squash merge
  changes it, so update the file after such a merge.
- **CI is not run yet.** The workflow runs on GitHub only after a push.

