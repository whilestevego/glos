//! Heavy runs over a big review, each held to a time budget so glos never quietly gets slower.
//!
//! The budgets are for a debug build (`cargo test`) with the suite running alongside, about
//! three times what the runs take on the machine they were set on. On a slower machine, scale
//! them with `GLOS_PERF_SCALE` (`GLOS_PERF_SCALE=2 cargo test`) instead of raising them here;
//! raise them here only for a change that's meant to cost more, and say so in its commit.
//!
//! A budget that loose catches a slowdown of twice or more (work done per key that should be
//! done once, an accidental quadratic), not a few percent: timing on a shared machine is too
//! noisy for that. The times printed per phase show where a failing run went.

use std::time::{Duration, Instant};

use super::*;

/// Fails if `run` takes longer than `budget` (times `GLOS_PERF_SCALE`); prints the time either
/// way, for `cargo test perf -- --nocapture`.
fn within<R>(name: &str, budget_ms: u64, run: impl FnOnce() -> R) -> R {
    let scale: f64 = std::env::var("GLOS_PERF_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let budget = Duration::from_millis(budget_ms).mul_f64(scale);
    let start = Instant::now();
    let out = run();
    let took = start.elapsed();
    eprintln!("perf: {name}: {took:?} (budget {budget:?})");
    assert!(
        took <= budget,
        "{name} took {took:?}, over its budget of {budget:?}"
    );
    out
}

/// A Rust-like file of `n` lines: functions of ten lines each, so syntax highlighting, text
/// objects and folding have real structure to work on. `edit` changes every 40th line.
fn source_file(n: usize, edit: bool) -> String {
    let mut s = String::new();
    for i in 0..n {
        let line = match i % 10 {
            0 => format!("fn item_{i}(x: u32) -> u32 {{"),
            1 => "    // Adds the parts up, then checks the total.".to_string(),
            2 => format!("    let a = x + {i};"),
            3 => "    let b = a * 2;".to_string(),
            4 => "    if b > 100 {".to_string(),
            5 => format!("        println!(\"big: {{}}\", b + {i});"),
            6 => "    }".to_string(),
            7 => "    let s = \"a string with some words in it\";".to_string(),
            8 => "    b + s.len() as u32".to_string(),
            _ => "}".to_string(),
        };
        if edit && i % 40 == 2 {
            s += &format!("    let a = x + {i} + 1; // changed\n");
        } else {
            s += &line;
            s.push('\n');
        }
    }
    s
}

/// Two folders of `files` changed files of `lines` lines each, opened on a 160×48 screen
/// with the file tree, as glos starts by default.
fn big_review(files: usize, lines: usize) -> T {
    let dir = tempfile::tempdir().unwrap();
    let (o, n) = (dir.path().join("old"), dir.path().join("new"));
    for i in 0..files {
        let sub = format!("src/mod_{}", i % 8);
        std::fs::create_dir_all(o.join(&sub)).unwrap();
        std::fs::create_dir_all(n.join(&sub)).unwrap();
        let path = format!("{sub}/file_{i:03}.rs");
        std::fs::write(o.join(&path), source_file(lines, false)).unwrap();
        std::fs::write(n.join(&path), source_file(lines, true)).unwrap();
    }
    let source = Source::diff(&o, &n).unwrap();
    T::new(source, dir, 160, 48, &Config::default())
}

#[test]
fn perf_opening_a_big_review() {
    // 150 files of 2,000 lines, each with 50 changes: the diff, the first file's syntax tree
    // and folds, the tree, the store and the first screen.
    let t = within("open 150 files × 2,000 lines", 5_000, || {
        big_review(150, 2_000)
    });
    assert!(t.shows("file_000.rs"));
}

/// The phases of the key run, each a few keys a reviewer presses, drawing after every key.
type Phase = (&'static str, fn(&mut T));

const PHASES: &[Phase] = &[
    ("lines, pages, ends", |t| {
        t.keys(&"j ".repeat(100));
        t.keys(&"C-d ".repeat(40));
        t.keys("G g g");
    }),
    ("next and previous change", |t| {
        t.keys(&"] g ".repeat(50));
        t.keys(&"[ g ".repeat(10));
    }),
    ("search", |t| {
        t.keys("/").typed("item_3999").keys("ret n n n N");
    }),
    ("selections and text objects", |t| {
        t.keys("x x x % ; m i f ; w w b e");
    }),
    ("unified and split views", |t| {
        t.keys("space v u C-d C-d ] g ] g space v s C-d ] g space v f");
    }),
    ("changed-file picker", |t| {
        t.keys("space g").typed("file_02").keys("ret");
    }),
    ("next file", |t| {
        t.keys(&"g n ".repeat(10));
    }),
    ("a comment", |t| {
        t.keys("x space c")
            .typed("Is this right?")
            .keys("esc ret ] g [ g");
    }),
];

#[test]
fn perf_keys_through_a_big_review() {
    let mut t = big_review(30, 4_000);
    within("keys through 30 files × 4,000 lines", 20_000, || {
        for (name, run) in PHASES {
            let start = Instant::now();
            run(&mut t);
            eprintln!("perf:   {name}: {:?}", start.elapsed());
        }
    });
    assert!(t.shows("Is this right?"));
}
