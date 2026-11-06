# glos

Plans live in `plans/`. `plans/README.md` indexes them by status and sets the naming rules.
`plans/completed/plan-overview.md` is the master plan and the map of glos. `plans/DECISIONS.md`
logs design decisions, and `plans/KEYMAP.md` is the generated keymap checklist.

## Writing plans

Follow the patterns the existing plans use. They are deliberate.

- Name a new plan `plans/plan-<topic>.md`, and add it to the "Active plans" section of
  `plans/README.md`.
- Open with a `**Status:**` line under the title, such as `Draft`, `Ready to build` or
  `In progress` with the branch name. Keep it current, and write dates as absolute dates.
- Make each plan stand on its own. Refer to another plan by its path from `plans/`, such as
  `completed/plan-tree.md`, and to the overview by section name. A reader who opens one plan by
  itself must be able to follow every reference.
- Tie each step to real code. Name the file and the function or type, such as
  `src/ui/tree.rs` `Tree::rebuilt`, rather than describing it vaguely.
- Say what tests prove each step is done. Every feature comes with broad tests.
- Record decisions so nobody argues them again. Put a lasting decision in `plans/DECISIONS.md`
  with the options considered, and mark anything unsettled as open.
- Treat the plan as the build record. When the work is built, add an "As built" section that
  says where the code departs from the plan and why. Then set the status to
  `Implemented (YYYY-MM-DD) in PR #N`, move the file into `plans/completed/` with `git mv`, and
  move its row in `plans/README.md` to "Completed".
- When a plan is dropped, move it into `plans/abandoned/`. Its status line gives the date, the
  reason and what would bring it back, and its row goes under "Abandoned" in the index.
- Don't cite a plan from a code comment. A comment states the constraint in place. Reasons,
  rejected options and history belong in `plans/`.

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
