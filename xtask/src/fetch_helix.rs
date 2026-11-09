//! `cargo xtask fetch-helix [lang …]`: copies `languages.toml`, the highlight queries and the
//! themes (MPL-2.0) from the pinned Helix release into `crates/glos-helix/helix/`, and each
//! grammar's C sources, at the revision Helix pins, into `crates/glos-helix/vendor/grammars/`.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Component, Path},
};

use anyhow::{Context, Result, bail};

use crate::{HELIX_TAG, fetch, root};

/// The languages glos highlights. Adding one: a line here, then rerun.
const LANGS: &[&str] = &[
    "rust",
    "typescript",
    "tsx",
    "javascript",
    "python",
    "go",
    "c",
    "cpp",
    "java",
    "ruby",
    "bash",
    "json",
    "toml",
    "yaml",
    "markdown",
    "markdown.inline",
    "html",
    "css",
    "sql",
    "lua",
    "dart",
    "swift",
    "kotlin",
];

const QUERY_FILES: &[&str] = &[
    "highlights.scm",
    "injections.scm",
    "locals.scm",
    "textobjects.scm",
];

/// A repository at a revision, from GitHub: each regular file by its path inside the repository.
fn tarball(repo: &str, rev: &str) -> Result<BTreeMap<String, Vec<u8>>> {
    let url = format!("{}/archive/{rev}.tar.gz", repo.trim_end_matches(".git"));
    let bytes = fetch(&url)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]));
    let mut files = BTreeMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?.to_string_lossy().into_owned();
        // Archives put everything under one top folder, `<repo>-<rev>/`.
        let rel = path.split_once('/').map_or("", |(_, r)| r).to_string();
        let mut content = vec![];
        std::io::Read::read_to_end(&mut entry, &mut content)?;
        files.insert(rel, content);
    }
    Ok(files)
}

/// Writes a file taken from an archive, refusing paths that climb out (it's untrusted).
fn write(dest: &Path, rel: &str, content: &[u8]) -> Result<()> {
    if Path::new(rel)
        .components()
        .any(|c| c == Component::ParentDir)
    {
        bail!("bad path in archive: {rel}");
    }
    fs::create_dir_all(dest.parent().unwrap())?;
    fs::write(dest, content)?;
    Ok(())
}

pub fn run(langs: &[String]) -> Result<()> {
    let langs: Vec<&str> = if langs.is_empty() {
        LANGS.to_vec()
    } else {
        langs.iter().map(String::as_str).collect()
    };
    let crate_dir = root().join("crates/glos-helix");
    let (out_helix, out_vendor) = (crate_dir.join("helix"), crate_dir.join("vendor/grammars"));

    let helix = tarball("https://github.com/helix-editor/helix", HELIX_TAG)?;
    let _ = fs::remove_dir_all(&out_helix);
    let file = |rel: &str| helix.get(rel).with_context(|| format!("no {rel} in Helix"));
    write(
        &out_helix.join("languages.toml"),
        "languages.toml",
        file("languages.toml")?,
    )?;
    write(
        &out_helix.join("themes/default.toml"),
        "theme.toml",
        file("theme.toml")?,
    )?;
    write(&out_helix.join("LICENSE"), "LICENSE", file("LICENSE")?)?;
    // Every theme Helix ships.
    for (rel, content) in &helix {
        if let Some(name) = rel.strip_prefix("runtime/themes/")
            && rel.ends_with(".toml")
        {
            write(&out_helix.join("themes").join(name), rel, content)?;
        }
    }

    let config: toml::Table = toml::from_str(&String::from_utf8_lossy(file("languages.toml")?))?;
    let table = |key: &str| -> Result<BTreeMap<String, toml::Table>> {
        let list = config[key].as_array().context(key.to_string())?;
        Ok(list
            .iter()
            .filter_map(|v| {
                let t = v.as_table()?;
                Some((t.get("name")?.as_str()?.to_string(), t.clone()))
            })
            .collect())
    };
    let (languages, grammars) = (table("language")?, table("grammar")?);

    // Queries, following `; inherits:` so every query a language needs is vendored.
    fn queries(
        lang: &str,
        seen: &mut HashSet<String>,
        helix: &BTreeMap<String, Vec<u8>>,
        out: &Path,
    ) -> Result<()> {
        if !seen.insert(lang.to_string()) {
            return Ok(());
        }
        for q in QUERY_FILES {
            let rel = format!("runtime/queries/{lang}/{q}");
            let Some(content) = helix.get(&rel) else {
                continue;
            };
            write(&out.join("queries").join(lang).join(q), &rel, content)?;
            for line in String::from_utf8_lossy(content).lines() {
                let s = line.trim_start_matches([';', ' ']).trim();
                if s.starts_with("inherits") {
                    let parents = s.split_once(':').map_or(s, |(_, p)| p);
                    for parent in parents.trim().split(',') {
                        queries(parent.trim(), seen, helix, out)?;
                    }
                }
            }
        }
        Ok(())
    }
    let mut seen = HashSet::new();
    for lang in &langs {
        queries(lang, &mut seen, &helix, &out_helix)?;
    }

    fs::write(
        out_helix.join("NOTICE"),
        format!(
            "Files in this folder are taken from Helix {HELIX_TAG} (https://github.com/helix-editor/helix)\n\
             and are licensed under the Mozilla Public License 2.0; see LICENSE.\n"
        ),
    )?;

    // Grammars: only <subpath>/src/ (parser.c, scanner.c, headers) plus a repository-level
    // common/ if present, keeping the repository's layout: scanners include ../../common.
    let _ = fs::remove_dir_all(&out_vendor);
    for lang in &langs {
        let language = languages
            .get(*lang)
            .with_context(|| format!("no language {lang}"))?;
        let name = language
            .get("grammar")
            .and_then(|g| g.as_str())
            .unwrap_or(lang);
        let source = grammars
            .get(name)
            .and_then(|g| g.get("source")?.as_table())
            .with_context(|| format!("no grammar source for {name}"))?;
        let get = |k: &str| source.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let (git, rev, sub) = (get("git"), get("rev"), get("subpath"));
        let repo = tarball(git, rev)?;
        let dest = out_vendor.join(name);
        let srcdir = if sub.is_empty() {
            "src/".to_string()
        } else {
            format!("{sub}/src/")
        };
        for (rel, content) in &repo {
            let licence =
                ["LICENSE", "LICENSE.md", "LICENSE.txt", "COPYING"].contains(&rel.as_str());
            let source_file = (rel.starts_with(&srcdir) || rel.starts_with("common/"))
                && [".c", ".h", ".cc"].iter().any(|e| rel.ends_with(e))
                // Swift ships parsers for older tree-sitter ABIs beside parser.c. build.rs
                // compiles only parser.c, and each copy is about 15 MB.
                && !rel[srcdir.len().min(rel.len())..].starts_with("parser_abi");
            if source_file || licence {
                write(&dest.join(rel), rel, content)?;
            }
        }
        fs::write(dest.join("REV"), format!("{git} {rev} {sub}\n"))?;
        eprintln!("vendored {name}");
    }
    Ok(())
}
