# glos

Plans and decisions live in `plans/` (`PLAN.md`, `DECISIONS.md`, `KEYMAP.md`).

## Commits and pull requests

- **Commit messages:** a subject line in plain English, sentence case, no trailing period, and no
  `feat:`/`fix:` prefixes; an `Area:` prefix is fine. Then a short body saying why. Examples from
  the history: "Reviews are named, never guessed", "Folding: unchanged code folds around changes,
  in every view", "Delete the M0 spikes; they're in git history".
- **PR titles are written the same way as commit subjects.** For a one-commit PR, the title is the
  commit's subject.
- **Before every commit:** `cargo fmt --all`, `cargo clippy --workspace --all-targets` with no
  warnings, and `cargo test --workspace`. Every feature comes with broad tests.
- Stage the whole repo, and check `git status` is clean after committing.
