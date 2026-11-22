# glos

The plans are in `plans/`. `plans/README.md` is the index of the plans by status, and it sets
the naming rules. `plans/completed/plan-overview.md` is the master plan and the map of glos.
`plans/DECISIONS.md` is the log of design decisions. `plans/KEYMAP.md` is the keymap checklist,
and `cargo xtask keymap` writes it.

## Engineering standard

Run `cargo xtask setup` once after you clone the repo. It installs the git hooks from
`.githooks/`, and it tells `git blame` to skip the commits in `.git-blame-ignore-revs`.

| Command | What it does |
|---|---|
| `cargo xtask ci` | Runs every check that CI runs. Run it before each commit. |
| `cargo xtask pre-commit` | Runs the format check and clippy. The pre-commit hook runs it. |
| `cargo xtask commit-msg <file>` | Checks the subject and the prose of a commit message. The commit-msg hook runs it. |
| `cargo xtask prose [base]` | Runs Vale on the lines that the branch adds since `base` (default `main`). |
| `cargo xtask prose-test` | Checks that each Vale rule fires on its line of `.vale/fixtures/fail.*`. |

- The lints are in `[workspace.lints]` in `Cargo.toml`. `clippy::pedantic` is on, and so are
  `unwrap_used`, `expect_used`, `print_stdout` and `print_stderr` outside tests. CI runs clippy
  with `-D warnings`.
- An exception to a lint is an `#[expect(lint, reason = "...")]` on the item that needs it. Do
  not use `#[allow]`. A clippy lint rejects it.
- `unsafe_code` is denied. Each `unsafe` site has an `#[expect(unsafe_code, reason = "...")]`
  and a `// SAFETY:` comment directly above the `unsafe` block.
- `rustfmt.toml` sets the format. A commit that only reformats goes in
  `.git-blame-ignore-revs`.
- The prose rules are in `.vale/styles/GLOS/`. They apply to Markdown files and to `//`, `///`
  and `//!` comments. Vale checks only the lines that a branch adds. The plans in
  `plans/completed/` and `plans/abandoned/` are history, and Vale does not check them.
- xtask downloads Vale into `target/vale/`, at the version and SHA-256 in `xtask/src/vale.rs`.
- `rust-toolchain.toml` sets the Rust version. Change it on purpose, and run `cargo xtask ci`
  on the new version first.

## Writing plans

Use the patterns of the existing plans. They are deliberate.

- Name a new plan `plans/plan-<topic>.md`. Add it to the "Active plans" section of
  `plans/README.md`.
- Put a `**Status:**` line under the title, such as `Draft`, `Ready to build` or
  `In progress` with the branch name. Keep it current. Write dates as absolute dates.
- Make each plan stand on its own. Refer to another plan by its path from `plans/`, such as
  `completed/plan-tree.md`. Refer to the overview by the name of its section. A reader who opens
  one plan alone must be able to follow each reference.
- Tie each step to real code. Name the file and the function or type, such as
  `src/ui/tree.rs` `Tree::rebuilt`.
- Say which tests prove that each step is done. Each feature comes with broad tests.
- Record decisions, so that nobody argues them again. Put a lasting decision in
  `plans/DECISIONS.md`, with the options that you considered. Mark each unsettled point as open.
- Use the plan as the record of the build. When the work is built, add an "As built" section.
  It says where the code departs from the plan, and why. Then set the status to
  `Implemented (YYYY-MM-DD) in PR #N`. Move the file into `plans/completed/` with `git mv`, and
  move its row in `plans/README.md` to "Completed".
- When you drop a plan, move it into `plans/abandoned/`. Its status line gives the date, the
  reason and what would bring it back. Its row goes under "Abandoned" in the index.
- Do not refer to a plan from a code comment. A comment states the constraint where it applies.
  Reasons, rejected options and history go in `plans/`.

## Commits and pull requests

- Write commit subjects in the Conventional Commits format of `~/hobby/redouble`:
  `type(scope): summary`. The commit-msg hook checks the format.
- The type is `feat`, `fix`, `refactor`, `docs`, `test`, `build` or `chore`. The scope is the
  area of glos, such as `tree`, `mouse`, `picker` or `plans`.
- Write the summary in the imperative, in lowercase, with no final period. Try to keep the
  subject to 60 characters or fewer. The hook rejects a subject of more than 72 characters.
- Add a short body that says why. Examples of subjects:
  `feat(mouse): focus the clicked panel and scroll under the pointer` and
  `docs(plans): plan mouse, tree, diff and comment work`.
- Commits from before November 2026 use an older style. Do not copy them.
- Write a PR title in the same way as a commit subject. For a PR of one commit, the title is the
  subject of the commit.
- Before each commit, run `cargo xtask ci`. Each feature comes with broad tests.
- Stage the whole repo. After the commit, check that `git status` is clean.
