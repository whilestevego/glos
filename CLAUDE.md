# glos

Plans and decisions live in `plans/` (`PLAN.md`, `DECISIONS.md`, `KEYMAP.md`).

## Commits and pull requests

- Commit subjects follow Conventional Commits, as in `~/hobby/redouble`: `type(scope): summary`.
  The type is `feat`, `fix`, `refactor`, `docs`, `test`, `build` or `chore`, and the scope is the
  area of glos, such as `tree`, `mouse`, `picker` or `plans`. Write the summary in the imperative,
  in lowercase, with no trailing period, and keep the whole subject under about 60 characters.
  Then add a short body that says why. Examples: "feat(mouse): focus the clicked panel and scroll
  under the pointer", "docs(plans): plan mouse, tree, diff and comment work". Commits made before
  November 2026 use an older style; don't copy them.
- PR titles are written the same way as commit subjects. For a one-commit PR, the title is the
  commit's subject.
- **Before every commit:** `cargo fmt --all`, `cargo clippy --workspace --all-targets` with no
  warnings, and `cargo test --workspace`. Every feature comes with broad tests.
- Stage the whole repo, and check `git status` is clean after committing.
