//! `cargo xtask keymap [default.rs]`: writes `plans/KEYMAP.md`, every key in Helix's default
//! normal-mode keymap at the pinned release and what glos does with it (done, replaced, not
//! applicable, or to do).
//!
//! Reads Helix's `helix-term/src/keymap/default.rs` (downloaded at the pinned tag, or a path
//! given) and glos's `src/ui/keymap.rs`. Rerun after changing the keymap or moving Helix release.

use std::{collections::HashSet, path::PathBuf};

use anyhow::{Context, Result};
use regex::Regex;

use crate::{HELIX_TAG, fetch, root};

/// Helix commands glos has no use for, by why: matched against the command's name.
const NOT_APPLICABLE: &[(&str, &str)] = &[
    (
        "editing comes after the MVP",
        r"^(insert|append|open_|delete|change|replace|paste|undo|redo|earlier|later|indent|unindent|format|join|switch_case|switch_to_|toggle_.*comments|add_newline|surround_(add|replace|delete)|increment|decrement|commit_undo|smart_tab|completion|kill_|dot_repeat|replace_selections_with|yank_joined|reflow|align_selections|rotate_selection_contents)",
    ),
    (
        "needs a language server",
        r"^(goto_(definition|declaration|type_definition|reference|implementation)|hover|rename_symbol|code_action|symbol_picker|workspace_symbol_picker|diagnostics_picker|workspace_diagnostics_picker|goto_(next|prev|first|last)_diag|select_references_to_symbol_under_cursor|signature_help|workspace_command_picker)$",
    ),
    (
        "glos has no buffers or windows",
        r"^(buffer_picker|goto_last_accessed_file|goto_last_modified_file|goto_last_modification|rotate_view|hsplit|vsplit|transpose_view|goto_file_[hv]split|wclose|wonly|jump_view_|swap_view_|hsplit_new|vsplit_new|goto_file|file_picker_in_current_directory|file_explorer_in_current_buffer_directory|last_picker)",
    ),
    ("no debugger", r"^dap_"),
    (
        "no shell commands on a read-only review",
        r"^(shell_|pipe|keep_pipe)",
    ),
];

fn re(s: &str) -> Regex {
    Regex::new(s).unwrap()
}

/// (key path, command) pairs from Helix's `keymap!` macro.
fn parse_helix(src: &str) -> Vec<(String, String)> {
    let menu = re(r#"^((?:"[^"]+"\s*\|\s*)*"[^"]+")\s*=>\s*\{\s*"([^"]+)""#);
    let bind = re(r#"^((?:"[^"]+"\s*\|\s*)*"[^"]+")\s*=>\s*([a-z_]+)"#);
    let quoted = re(r#""([^"]+)""#);
    let (mut out, mut stack) = (vec![], vec![]);
    for line in src.lines() {
        let line = line.split("//").next().unwrap().trim();
        if let Some(m) = menu.captures(line) {
            stack.push(quoted.captures(&m[1]).unwrap()[1].to_string());
        } else if let Some(m) = bind.captures(line) {
            for key in quoted.captures_iter(&m[1]) {
                let path = stack.iter().cloned().chain([key[1].to_string()]);
                out.push((path.collect::<Vec<_>>().join(" "), m[2].to_string()));
            }
        } else if line.starts_with('}') {
            stack.pop();
        }
    }
    out
}

/// (key path, command) pairs from `normal_defs()` in glos's keymap, with helper menus like
/// `view_defs()` expanded.
/// The contexts glos adds beside normal mode, as the checklist titles them, and the function in
/// `keymap.rs` that lists each one's keys.
const CONTEXTS: &[(&str, &str)] = &[
    ("the file tree", "tree_defs"),
    ("pickers", "picker_defs"),
    ("the command line", "prompt_defs"),
    ("the comment box in normal mode", "box_normal_defs"),
    ("the comment box in insert mode", "box_insert_defs"),
];

/// The keys that `defs` (such as `normal_defs`) binds in glos's `keymap.rs`, as key paths.
fn parse_glos(src: &str, defs: &str) -> Vec<(String, String)> {
    let defs_of = |f: &str| -> String {
        let start = src.find(&format!("fn {f}()")).expect(f);
        let end = start + src[start..].find("\n}\n").expect(f);
        // rustfmt splits a menu's arguments over lines; join them back.
        re(r#"((?:sticky_)?menu)\(\s*("[^"]+"),\s*("[^"]+"),\s*"#)
            .replace_all(&src[start..end], "$1($2, $3, ")
            .into_owned()
    };
    let helper = re(r#"^(?:sticky_)?menu\("([^"]+)",\s*"[^"]+",\s*([a-z_]+)\(\)\)"#);
    let menu = re(r#"^(?:sticky_)?menu\("([^"]+)",\s*"[^"]+",\s*vec!\["#);
    let cmd = re(r#"^cmd\("([^"]+)",\s*"([a-z_]+)"\)"#);
    fn walk(
        body: &str,
        mut stack: Vec<String>,
        out: &mut Vec<(String, String)>,
        res: (&Regex, &Regex, &Regex),
        defs_of: &dyn Fn(&str) -> String,
    ) {
        let (helper, menu, cmd) = res;
        for line in body.lines().map(str::trim) {
            if let Some(m) = helper.captures(line) {
                let mut inner = stack.clone();
                inner.push(m[1].to_string());
                walk(&defs_of(&m[2]), inner, out, res, defs_of);
            } else if let Some(m) = menu.captures(line) {
                stack.push(m[1].to_string());
            } else if let Some(m) = cmd.captures(line) {
                let keys: Vec<&str> = if &m[1] == "|" {
                    vec!["|"]
                } else {
                    m[1].split('|').collect()
                };
                for key in keys {
                    let path = stack.iter().map(String::as_str).chain([key]);
                    out.push((path.collect::<Vec<_>>().join(" "), m[2].to_string()));
                }
            } else if line.starts_with(']') {
                stack.pop();
            }
        }
    }
    let mut out = vec![];
    walk(
        &defs_of(defs),
        vec![],
        &mut out,
        (&helper, &menu, &cmd),
        &defs_of,
    );
    // A key bound twice keeps its first place and its last command, as a map would.
    let mut keyed: Vec<(String, String)> = vec![];
    for (k, c) in out {
        match keyed.iter_mut().find(|(key, _)| *key == k) {
            Some(e) => e.1 = c,
            None => keyed.push((k, c)),
        }
    }
    keyed
}

/// The checklist, as markdown, and the counts by status in the order first seen.
fn checklist(helix_src: &str, glos_src: &str) -> (String, Vec<(String, usize)>) {
    // Only the normal-mode map: select mode is the same keys extending, insert mode needs editing.
    let start = helix_src
        .find("let normal = keymap!")
        .expect("Helix's normal keymap");
    let end = helix_src
        .find("let mut select")
        .expect("Helix's select keymap");
    let helix = parse_helix(&helix_src[start..end]);
    let glos = parse_glos(glos_src, "normal_defs");
    let glos_cmd = |k: &str| {
        glos.iter()
            .find(|(key, _)| key == k)
            .map(|(_, c)| c.as_str())
    };
    let not_applicable: Vec<(&str, Regex)> = NOT_APPLICABLE
        .iter()
        .map(|(why, pat)| (*why, re(pat)))
        .collect();

    let (mut rows, mut counts) = (vec![], Vec::<(String, usize)>::new());
    for (key, cmd) in &helix {
        let g = glos_cmd(key);
        let status = if g == Some(cmd.as_str()) {
            "done".to_string()
        } else if let Some(g) = g {
            format!("replaced: `{g}`")
        } else if let Some((why, _)) = not_applicable.iter().find(|(_, p)| p.is_match(cmd)) {
            format!("not applicable: {why}")
        } else {
            "to do".to_string()
        };
        let kind = status.split(':').next().unwrap().to_string();
        match counts.iter_mut().find(|(k, _)| *k == kind) {
            Some(c) => c.1 += 1,
            None => counts.push((kind, 1)),
        }
        rows.push(format!(
            "| `{}` | `{cmd}` | {status} |",
            key.replace('|', "\\|")
        ));
    }
    let helix_keys: HashSet<&str> = helix.iter().map(|(k, _)| k.as_str()).collect();
    let extra = glos
        .iter()
        .filter(|(k, _)| !helix_keys.contains(k.as_str()))
        .map(|(k, c)| format!("| `{k}` | `{c}` |"));

    let mut by_count = counts.clone();
    by_count.sort_by(|a, b| b.1.cmp(&a.1)); // stable: ties keep the order first seen
    let summary: Vec<String> = by_count.iter().map(|(s, n)| format!("{n} {s}")).collect();
    let mut out = vec![
        "# Keymap checklist".to_string(),
        String::new(),
        format!(
            "Generated by `cargo xtask keymap` from Helix {HELIX_TAG}'s default normal-mode keymap"
        ),
        "and glos's `src/ui/keymap.rs`. Select mode is the same keys extending the selection; insert mode".into(),
        "comes with editing. Don't edit by hand: rerun the task.".into(),
        String::new(),
        format!("Summary: {}.", summary.join(", ")),
        String::new(),
        "| Key | Helix command | glos |".into(),
        "|---|---|---|".into(),
    ];
    out.extend(rows);
    out.extend([
        String::new(),
        "## Keys glos adds".into(),
        String::new(),
        "| Key | glos command |".into(),
        "|---|---|".into(),
    ]);
    out.extend(extra);
    // The other contexts have no Helix keymap to compare with, so they're only listed.
    for (title, defs) in CONTEXTS {
        if !glos_src.contains(&format!("fn {defs}()")) {
            continue;
        }
        out.extend([
            String::new(),
            format!("## Keys in {title}"),
            String::new(),
            "| Key | glos command |".into(),
            "|---|---|".into(),
        ]);
        out.extend(
            parse_glos(glos_src, defs)
                .iter()
                .map(|(k, c)| format!("| `{}` | `{c}` |", k.replace('|', "\\|"))),
        );
    }
    out.push(String::new());
    (out.join("\n"), counts)
}

pub fn run(helix_keymap: Option<PathBuf>) -> Result<()> {
    let helix_src = match helix_keymap {
        Some(p) => std::fs::read_to_string(&p).with_context(|| p.display().to_string())?,
        None => String::from_utf8(fetch(&format!(
            "https://raw.githubusercontent.com/helix-editor/helix/{HELIX_TAG}/helix-term/src/keymap/default.rs"
        ))?)?,
    };
    let glos_src = std::fs::read_to_string(root().join("src/ui/keymap.rs"))?;
    let (md, counts) = checklist(&helix_src, &glos_src);
    std::fs::write(root().join("plans/KEYMAP.md"), md)?;
    let counts: Vec<String> = counts.iter().map(|(s, n)| format!("{n} {s}")).collect();
    println!("wrote plans/KEYMAP.md: {}", counts.join(", "));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELIX: &str = r#"
    let normal = keymap!({ "Normal mode"
        "h" | "left" => move_char_left,
        "i" => insert_mode,
        "g" => { "Goto"
            "g" => goto_file_start,
            "d" => goto_definition, // an LSP one
        },
        "space" => { "Space"
            "c" => toggle_comments,
            "w" => { "Window"
                "v" => vsplit,
            },
        },
        "]" => { "Right"
            "f" => goto_next_function,
        },
    });
    let mut select = normal.clone();
    "#;

    const GLOS: &str = r#"
fn normal_defs() -> Vec<Def> {
    vec![
        cmd("h|left", "move_char_left"),
        menu("g", "Goto", vec![
            cmd("g", "goto_file_start"),
        ]),
        menu(
            "space",
            "Space",
            vec![
                cmd("c", "comment"),
                cmd("l", "dashboard"),
            ],
        ),
        menu("z", "View", view_defs()),
    ]
}

fn view_defs() -> Vec<Def> {
    vec![
        cmd("z|c", "align_view_center"),
    ]
}

fn tree_defs() -> Vec<Def> {
    vec![
        cmd("j|down", "tree_next"),
    ]
}
"#;

    #[test]
    fn both_keymaps_parse_into_key_paths() {
        assert_eq!(
            parse_helix(HELIX),
            [
                ("h", "move_char_left"),
                ("left", "move_char_left"),
                ("i", "insert_mode"),
                ("g g", "goto_file_start"),
                ("g d", "goto_definition"),
                ("space c", "toggle_comments"),
                ("space w v", "vsplit"),
                ("] f", "goto_next_function"),
            ]
            .map(|(k, c)| (k.to_string(), c.to_string()))
        );
        let glos = parse_glos(GLOS, "normal_defs");
        let keys: Vec<&str> = glos.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            ["h", "left", "g g", "space c", "space l", "z z", "z c"],
            "menus split over lines by rustfmt, and helper menus, are followed"
        );
    }

    #[test]
    fn every_helix_key_gets_a_status() {
        let (md, counts) = checklist(HELIX, GLOS);
        for row in [
            "| `h` | `move_char_left` | done |",
            "| `i` | `insert_mode` | not applicable: editing comes after the MVP |",
            "| `g d` | `goto_definition` | not applicable: needs a language server |",
            "| `space c` | `toggle_comments` | replaced: `comment` |",
            "| `space w v` | `vsplit` | not applicable: glos has no buffers or windows |",
            "| `] f` | `goto_next_function` | to do |",
            "| `space l` | `dashboard` |",
        ] {
            assert!(md.contains(row), "{row} missing from:\n{md}");
        }
        assert!(
            md.contains("Summary: 3 done, 3 not applicable, 1 replaced, 1 to do."),
            "{md}"
        );
        assert_eq!(counts[0], ("done".to_string(), 3));
        // Each context glos adds gets a table, after the keys glos adds to normal mode.
        let tree = md
            .find("## Keys in the file tree")
            .expect("a table for the tree");
        assert!(md.find("## Keys glos adds").unwrap() < tree);
        assert!(
            md[tree..].contains("| `j` | `tree_next` |\n| `down` | `tree_next` |"),
            "{md}"
        );
        assert!(
            !md.contains("## Keys in pickers"),
            "no picker_defs in the fixture"
        );
    }
}
