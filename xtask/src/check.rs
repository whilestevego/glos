//! The checks that the git hooks run, and the task that installs the hooks.

use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::{prose, root, vale};

/// The commit types that `CLAUDE.md` allows.
const TYPES: &[&str] = &["feat", "fix", "refactor", "docs", "test", "build", "chore"];

/// Points git at the versioned hooks in `.githooks/` and at `.git-blame-ignore-revs`.
pub fn setup() -> Result<()> {
    git_config("core.hooksPath", ".githooks")?;
    git_config("blame.ignoreRevsFile", ".git-blame-ignore-revs")?;
    vale::command()?;
    println!("git hooks installed from .githooks/");
    Ok(())
}

/// The format check and clippy.
pub fn pre_commit() -> Result<()> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])
}

/// Everything CI runs: the format check, clippy, the tests, the prose lint on the lines added
/// since `base`, and the prose self-test. It stops at the first failure.
pub fn ci(base: &str) -> Result<()> {
    pre_commit()?;
    cargo(&["test", "--workspace"])?;
    println!("$ cargo xtask prose {base}");
    prose::run(base)?;
    println!("$ cargo xtask prose-test");
    prose::self_test()
}

/// Checks the subject of the commit message in `path`, then the prose of the whole message.
pub fn commit_msg(path: &str) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    let message = strip_comments(&text);
    let subject = message.lines().next().unwrap_or_default();
    if let Err(e) = check_subject(subject) {
        bail!("{e}\n  subject: {subject}");
    }
    let tmp = root().join("target/COMMIT_EDITMSG.md");
    std::fs::write(&tmp, &message)?;
    let alerts = prose::alerts(&[&tmp])?;
    if !alerts.is_empty() {
        for a in &alerts {
            println!("{a}");
        }
        bail!("the commit message has {} prose alerts", alerts.len());
    }
    Ok(())
}

/// Runs cargo in the workspace root. Git sets `GIT_DIR` and `GIT_INDEX_FILE` for a hook, and
/// cargo's own git fetches must not see them.
pub fn cargo(args: &[&str]) -> Result<()> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    println!("$ cargo {}", args.join(" "));
    let status = Command::new(cargo)
        .args(args)
        .current_dir(root())
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_WORK_TREE")
        .status()?;
    if !status.success() {
        bail!("cargo {} failed", args.join(" "));
    }
    Ok(())
}

fn git_config(key: &str, value: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["config", key, value])
        .current_dir(root())
        .status()?;
    if !status.success() {
        bail!("git config {key} {value} failed");
    }
    Ok(())
}

/// The message without the comment lines that git adds, and without the diff that
/// `git commit --verbose` adds below its scissors line.
fn strip_comments(text: &str) -> String {
    text.lines()
        .take_while(|l| !l.starts_with("# ------------------------ >8 ------------------------"))
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// `type(scope): summary`, lowercase after the colon, no final period, and 72 characters or
/// fewer. Subjects that git writes itself pass.
fn check_subject(subject: &str) -> Result<(), String> {
    const GIT: &[&str] = &["Merge ", "Revert ", "fixup! ", "squash! ", "amend! "];
    if GIT.iter().any(|p| subject.starts_with(p)) {
        return Ok(());
    }
    let format = || {
        format!(
            "the subject must be `type(scope): summary`, with a type from: {}",
            TYPES.join(", ")
        )
    };
    let (head, summary) = subject.split_once(": ").ok_or_else(format)?;
    let (kind, scope) = match head.split_once('(') {
        Some((kind, rest)) => (kind, Some(rest.strip_suffix(')').ok_or_else(format)?)),
        None => (head, None),
    };
    if !TYPES.contains(&kind) {
        return Err(format());
    }
    if scope.is_some_and(|s| {
        s.is_empty()
            || !s
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-,".contains(&b))
    }) {
        return Err("the scope must be lowercase letters, digits, `-` or `,`".into());
    }
    match summary.chars().next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '`' => {},
        _ => return Err("the summary must start in lowercase".into()),
    }
    if summary.ends_with('.') {
        return Err("the subject must not end with a period".into());
    }
    let n = subject.chars().count();
    if n > 72 {
        return Err(format!(
            "the subject has {n} characters, and the limit is 72"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_passes_with_and_without_a_scope() {
        for t in TYPES {
            assert_eq!(check_subject(&format!("{t}: add a thing")), Ok(()));
            assert_eq!(check_subject(&format!("{t}(tree): add a thing")), Ok(()));
        }
    }

    #[test]
    fn a_scope_list_and_a_code_span_pass() {
        assert_eq!(check_subject("fix(ui,mcp): stop a crash"), Ok(()));
        assert_eq!(check_subject("docs(plans): move `plan-tree.md`"), Ok(()));
        assert_eq!(check_subject("build: pin the toolchain to 1.94.0"), Ok(()));
    }

    #[test]
    fn an_unknown_type_fails() {
        assert!(check_subject("style: reformat").is_err());
        assert!(check_subject("Feat: add a thing").is_err());
    }

    #[test]
    fn a_bad_scope_fails() {
        assert!(check_subject("feat(Tree): add a thing").is_err());
        assert!(check_subject("feat(): add a thing").is_err());
        assert!(check_subject("feat(tree: add a thing").is_err());
    }

    #[test]
    fn a_missing_colon_fails() {
        assert!(check_subject("feat add a thing").is_err());
        assert!(check_subject("feat:add a thing").is_err());
    }

    #[test]
    fn an_uppercase_summary_fails() {
        assert!(check_subject("feat(tree): Add a thing").is_err());
    }

    #[test]
    fn a_final_period_fails() {
        assert!(check_subject("feat(tree): add a thing.").is_err());
    }

    #[test]
    fn the_limit_is_72_characters() {
        let at = |n: usize| format!("feat: {}", "a".repeat(n - "feat: ".len()));
        assert_eq!(check_subject(&at(72)), Ok(()));
        assert!(check_subject(&at(73)).is_err());
    }

    #[test]
    fn subjects_that_git_writes_pass() {
        for s in [
            "Merge branch 'main' into topic",
            "Revert \"feat: add a thing\"",
            "fixup! feat: add a thing",
            "squash! feat: add a thing",
            "amend! feat: add a thing",
        ] {
            assert_eq!(check_subject(s), Ok(()), "{s}");
        }
    }

    #[test]
    fn comments_and_the_verbose_diff_are_stripped() {
        let text = "feat: add a thing\n\nThe body.\n# Please enter the message.\n\
                    # ------------------------ >8 ------------------------\ndiff --git a b\n";
        assert_eq!(strip_comments(text), "feat: add a thing\n\nThe body.");
    }
}
