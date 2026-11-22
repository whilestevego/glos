//! The prose lint: Vale on Markdown files and Rust comments, with the rules in `.vale/styles/GLOS/`.

use std::{collections::HashSet, process::Command};

use anyhow::{Context, Result, bail};

use crate::{root, vale};

/// Paths that the prose lint skips: history, generated text, test fixtures and vendored files.
const SKIPPED: &[&str] = &[
    "plans/completed/",
    "plans/abandoned/",
    "plans/KEYMAP.md",
    ".vale/fixtures/",
    "crates/glos-helix/vendor/",
    "crates/glos-helix/helix/",
];

/// The alert that each rule must give on the fail fixtures: (file, line, rule).
const EXPECTED: &[(&str, usize, &str)] = &[
    ("fail.md", 3, "GLOS.Dashes"),
    ("fail.md", 5, "GLOS.Banned"),
    ("fail.md", 7, "GLOS.SentenceLength"),
    ("fail.md", 9, "GLOS.Structures"),
    ("fail.md", 11, "GLOS.Structures"),
    ("fail.md", 13, "GLOS.Structures"),
    ("fail.rs", 1, "GLOS.Dashes"),
    ("fail.rs", 3, "GLOS.Banned"),
    ("fail.rs", 6, "GLOS.SentenceLength"),
    ("fail.rs", 11, "GLOS.Structures"),
];

/// Lints the lines that this branch adds since it left `base`, in the working tree.
pub fn run(base: &str) -> Result<()> {
    let merge_base = git(&["merge-base", base, "HEAD"])?;
    let merge_base = merge_base.trim();
    let files: Vec<String> = git(&[
        "diff",
        "--name-only",
        "--diff-filter=ACMR",
        merge_base,
        "--",
        "*.md",
        "*.rs",
    ])?
    .lines()
    .filter(|f| !SKIPPED.iter().any(|s| f.starts_with(s)))
    .map(String::from)
    .collect();
    if files.is_empty() {
        return Ok(());
    }
    let mut args = vec!["diff", "-U0", merge_base, "--"];
    args.extend(files.iter().map(String::as_str));
    let added = added_lines(&git(&args)?);
    let hits: Vec<String> = alerts(&files)?
        .into_iter()
        .filter(|a| {
            let mut parts = a.splitn(3, ':');
            let (file, line) = (parts.next(), parts.next().and_then(|l| l.parse().ok()));
            file.zip(line)
                .is_some_and(|(f, l): (&str, usize)| added.contains(&(f.to_string(), l)))
        })
        .collect();
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        println!("{h}");
    }
    bail!("{} prose alerts on lines this branch adds", hits.len())
}

/// Checks that the pass fixtures give no alerts and that each rule fires where `EXPECTED` says.
pub fn self_test() -> Result<()> {
    let pass = alerts(&[".vale/fixtures/pass.md", ".vale/fixtures/pass.rs"])?;
    if !pass.is_empty() {
        bail!("the pass fixtures give alerts:\n{}", pass.join("\n"));
    }
    let fail = alerts(&[".vale/fixtures/fail.md", ".vale/fixtures/fail.rs"])?;
    let got: HashSet<(String, usize, String)> = fail
        .iter()
        .filter_map(|a| {
            let mut p = a.split(':');
            let file = p.next()?.rsplit('/').next()?.to_string();
            let line = p.next()?.parse().ok()?;
            let rule = p.nth(1)?.to_string();
            Some((file, line, rule))
        })
        .collect();
    let want: HashSet<(String, usize, String)> = EXPECTED
        .iter()
        .map(|&(f, l, r)| (f.to_string(), l, r.to_string()))
        .collect();
    let missing: Vec<_> = want.difference(&got).collect();
    let extra: Vec<_> = got.difference(&want).collect();
    if !missing.is_empty() || !extra.is_empty() {
        bail!("prose self-test: missing alerts {missing:?}, unexpected alerts {extra:?}");
    }
    println!("prose self-test: {} rules fire where expected", want.len());
    Ok(())
}

/// Vale's alerts on `files`, one `path:line:col:rule:message` per entry.
pub fn alerts<S: AsRef<std::ffi::OsStr>>(files: &[S]) -> Result<Vec<String>> {
    let out = vale::command()?.arg("--output=line").args(files).output()?;
    if out.status.code().is_none_or(|c| c > 1) {
        bail!("vale failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8(out.stdout)?
        .lines()
        .map(String::from)
        .collect())
}

/// The (path, line) pairs that a `git diff -U0` adds.
fn added_lines(diff: &str) -> HashSet<(String, usize)> {
    let mut out = HashSet::new();
    let mut file = None;
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            file = path.strip_prefix("b/").map(String::from);
        } else if let (Some(f), Some(hunk)) = (&file, line.strip_prefix("@@ ")) {
            let Some(new) = hunk.split(' ').find_map(|p| p.strip_prefix('+')) else {
                continue;
            };
            let (start, len) = new.split_once(',').unwrap_or((new, "1"));
            let (Ok(start), Ok(len)) = (start.parse::<usize>(), len.parse::<usize>()) else {
                continue;
            };
            out.extend((start..start + len).map(|l| (f.clone(), l)));
        }
    }
    out
}

fn git(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root())
        .output()
        .context("run git")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8(out.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(diff: &str) -> Vec<(String, usize)> {
        let mut v: Vec<_> = added_lines(diff).into_iter().collect();
        v.sort();
        v
    }

    #[test]
    fn a_hunk_of_one_line() {
        let diff = "+++ b/a.md\n@@ -3 +4 @@\n-old\n+new\n";
        assert_eq!(lines(diff), [("a.md".into(), 4)]);
    }

    #[test]
    fn a_hunk_of_many_lines() {
        let diff = "+++ b/a.md\n@@ -1,0 +2,3 @@\n+x\n+y\n+z\n";
        assert_eq!(
            lines(diff),
            [("a.md".into(), 2), ("a.md".into(), 3), ("a.md".into(), 4)]
        );
    }

    #[test]
    fn a_hunk_that_only_removes_adds_nothing() {
        let diff = "+++ b/a.md\n@@ -5,2 +4,0 @@\n-x\n-y\n";
        assert!(lines(diff).is_empty());
    }

    #[test]
    fn a_new_file_and_a_second_file() {
        let diff = "--- /dev/null\n+++ b/new.rs\n@@ -0,0 +1,2 @@\n+a\n+b\n\
                    --- a/old.md\n+++ b/old.md\n@@ -9 +9 @@\n-c\n+d\n";
        assert_eq!(
            lines(diff),
            [
                ("new.rs".into(), 1),
                ("new.rs".into(), 2),
                ("old.md".into(), 9)
            ]
        );
    }

    #[test]
    fn a_deleted_file_adds_nothing() {
        let diff = "--- a/gone.md\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\n";
        assert!(lines(diff).is_empty());
    }

    #[test]
    fn every_expected_alert_names_a_fixture_and_a_rule() {
        for (file, line, rule) in EXPECTED {
            assert!(file.starts_with("fail."), "{file}");
            assert!(*line > 0, "{file}");
            assert!(rule.starts_with("GLOS."), "{rule}");
        }
    }
}
