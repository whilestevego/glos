//! Compiles every vendored grammar into the binary and embeds Helix's queries and themes.
use std::{env, fmt::Write, fs, path::Path};

fn main() {
    let out = env::var("OUT_DIR").unwrap();
    let mut code = String::new();

    // Grammars: vendor/grammars/<name>/[<subpath>/]src/parser.c
    let mut names = Vec::new();
    for entry in fs::read_dir("vendor/grammars").unwrap() {
        let dir = entry.unwrap().path();
        let name = dir.file_name().unwrap().to_str().unwrap().to_string();
        let rev = fs::read_to_string(dir.join("REV")).unwrap();
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
        writeln!(code, "    fn tree_sitter_{n}() -> *const ();").unwrap();
    }
    code.push_str("}\n/// (grammar name, language fn)\npub static GRAMMARS: &[(&str, unsafe extern \"C\" fn() -> *const ())] = &[\n");
    for n in &names {
        writeln!(code, "    (\"{n}\", tree_sitter_{n}),").unwrap();
    }
    code.push_str("];\n");

    // Queries and themes, embedded as text.
    let root = env::var("CARGO_MANIFEST_DIR").unwrap();
    code.push_str(
        "/// (language, file name, text)\npub static QUERIES: &[(&str, &str, &str)] = &[\n",
    );
    for lang in fs::read_dir("helix/queries").unwrap() {
        let lang = lang.unwrap().path();
        for q in fs::read_dir(&lang).unwrap() {
            let q = q.unwrap().path();
            writeln!(
                code,
                "    ({:?}, {:?}, include_str!({:?})),",
                lang.file_name().unwrap(),
                q.file_name().unwrap(),
                Path::new(&root).join(&q)
            )
            .unwrap();
        }
    }
    code.push_str("];\npub static THEMES: &[(&str, &str)] = &[\n");
    for t in fs::read_dir("helix/themes").unwrap() {
        let t = t.unwrap().path();
        writeln!(
            code,
            "    ({:?}, include_str!({:?})),",
            t.file_stem().unwrap(),
            Path::new(&root).join(&t)
        )
        .unwrap();
    }
    code.push_str("];\n");
    writeln!(
        code,
        "pub static LANGUAGES_TOML: &str = include_str!({:?});",
        Path::new(&root).join("helix/languages.toml")
    )
    .unwrap();
    println!("cargo:rerun-if-changed=helix");
    fs::write(Path::new(&out).join("embedded.rs"), code).unwrap();
}
