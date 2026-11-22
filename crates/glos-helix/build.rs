//! Compiles every vendored grammar into the binary and embeds Helix's queries and themes.
use std::{env, fmt::Write, fs, path::Path};

#[expect(
    clippy::unnecessary_debug_formatting,
    reason = "`{:?}` writes each path as a Rust string literal for the generated code"
)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = env::var("OUT_DIR")?;
    let mut code = String::new();

    // Grammars: vendor/grammars/<name>/[<subpath>/]src/parser.c
    let mut names = Vec::new();
    for entry in fs::read_dir("vendor/grammars")? {
        let dir = entry?.path();
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("a grammar folder without a UTF-8 name")?
            .to_string();
        let rev = fs::read_to_string(dir.join("REV"))?;
        let sub = rev.split_whitespace().nth(2).unwrap_or("");
        let src = dir.join(sub).join("src");
        let mut build = cc::Build::new();
        build
            .include(&src)
            .warnings(false)
            .opt_level(2)
            .flag_if_supported("-std=c11");
        build.file(src.join("parser.c"));
        if src.join("scanner.c").exists() {
            build.file(src.join("scanner.c"));
        }
        build.compile(&format!("tree-sitter-{name}"));
        // Older grammars (ruby, yaml at Helix's pins) have a C++ scanner.
        if src.join("scanner.cc").exists() {
            cc::Build::new()
                .cpp(true)
                .include(&src)
                .warnings(false)
                .opt_level(2)
                .flag_if_supported("-std=c++14")
                .file(src.join("scanner.cc"))
                .compile(&format!("tree-sitter-{name}-scanner"));
        }
        println!("cargo:rerun-if-changed={}", dir.display());
        names.push(name);
    }
    names.sort();
    code.push_str("unsafe extern \"C\" {\n");
    for n in &names {
        writeln!(code, "    fn tree_sitter_{n}() -> *const ();")?;
    }
    code.push_str("}\n/// (grammar name, language fn)\npub static GRAMMARS: &[(&str, unsafe extern \"C\" fn() -> *const ())] = &[\n");
    for n in &names {
        writeln!(code, "    (\"{n}\", tree_sitter_{n}),")?;
    }
    code.push_str("];\n");

    // Queries and themes, embedded as text.
    let root = env::var("CARGO_MANIFEST_DIR")?;
    code.push_str(
        "/// (language, file name, text)\npub static QUERIES: &[(&str, &str, &str)] = &[\n",
    );
    for lang in fs::read_dir("helix/queries")? {
        let lang = lang?.path();
        for q in fs::read_dir(&lang)? {
            let q = q?.path();
            writeln!(
                code,
                "    ({:?}, {:?}, include_str!({:?})),",
                lang.file_name().ok_or("a path without a file name")?,
                q.file_name().ok_or("a path without a file name")?,
                Path::new(&root).join(&q)
            )?;
        }
    }
    code.push_str("];\npub static THEMES: &[(&str, &str)] = &[\n");
    for t in fs::read_dir("helix/themes")? {
        let t = t?.path();
        writeln!(
            code,
            "    ({:?}, include_str!({:?})),",
            t.file_stem().ok_or("a path without a file name")?,
            Path::new(&root).join(&t)
        )?;
    }
    code.push_str("];\n");
    writeln!(
        code,
        "pub static LANGUAGES_TOML: &str = include_str!({:?});",
        Path::new(&root).join("helix/languages.toml")
    )?;
    println!("cargo:rerun-if-changed=helix");
    fs::write(Path::new(&out).join("embedded.rs"), code)?;
    Ok(())
}
