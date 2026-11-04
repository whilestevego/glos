#!/usr/bin/env python3
"""Vendor what glos takes from a pinned Helix release (rerun to move to a new one).

Copies languages.toml, highlight queries and themes (MPL-2.0) into helix/,
and each grammar's C sources, at the revision Helix pins, into vendor/grammars/.
Usage: scripts/fetch-helix.py [lang ...]   (defaults to LANGS below)
"""
import io, shutil, sys, tarfile, tomllib, urllib.request
from pathlib import Path

HELIX_TAG = "25.07.1"
# The languages glos highlights (plans/PLAN.md, Helix fidelity). Adding one: a line here, then rerun.
LANGS = sys.argv[1:] or [
    "rust", "typescript", "tsx", "javascript", "python", "go", "c", "cpp", "java", "ruby",
    "bash", "json", "toml", "yaml", "markdown", "markdown.inline", "html", "css", "sql", "lua",
]
QUERY_FILES = ["highlights.scm", "injections.scm", "locals.scm"]

root = Path(__file__).resolve().parent.parent
out_helix, out_vendor = root / "helix", root / "vendor" / "grammars"


def tarball(repo: str, rev: str) -> tarfile.TarFile:
    url = f"{repo.removesuffix('.git')}/archive/{rev}.tar.gz"
    print("fetch", url, file=sys.stderr)
    return tarfile.open(fileobj=io.BytesIO(urllib.request.urlopen(url).read()))


def members(tar: tarfile.TarFile, prefix: str):
    """(path relative to prefix, member) for regular files under prefix/."""
    for m in tar.getmembers():
        rel = m.name.split("/", 1)[1] if "/" in m.name else ""
        if m.isfile() and rel.startswith(prefix):
            yield rel, m


def extract(tar, rel, m, dest: Path):
    if ".." in Path(rel).parts:  # untrusted archive
        raise SystemExit(f"bad path in archive: {rel}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(tar.extractfile(m).read())


helix = tarball("https://github.com/helix-editor/helix", HELIX_TAG)
shutil.rmtree(out_helix, ignore_errors=True)
files = dict((rel, m) for rel, m in members(helix, ""))
extract(helix, "languages.toml", files["languages.toml"], out_helix / "languages.toml")
extract(helix, "theme.toml", files["theme.toml"], out_helix / "themes" / "default.toml")
extract(helix, "LICENSE", files["LICENSE"], out_helix / "LICENSE")
for rel, m in files.items():  # every theme Helix ships
    if rel.startswith("runtime/themes/") and rel.endswith(".toml"):
        extract(helix, rel, m, out_helix / "themes" / rel.removeprefix("runtime/themes/"))

config = tomllib.loads((out_helix / "languages.toml").read_text())
languages = {l["name"]: l for l in config["language"]}
grammars = {g["name"]: g for g in config["grammar"]}


# Queries, following `; inherits:` so every query a language needs is vendored.
def vendor_queries(lang: str, seen: set):
    if lang in seen:
        return
    seen.add(lang)
    for q in QUERY_FILES:
        rel = f"runtime/queries/{lang}/{q}"
        if rel in files:
            dest = out_helix / "queries" / lang / q
            extract(helix, rel, files[rel], dest)
            for line in dest.read_text().splitlines():
                s = line.lstrip("; ").strip()
                if s.startswith("inherits"):
                    for parent in s.split(":", 1)[-1].strip().split(","):
                        vendor_queries(parent.strip(), seen)


seen = set()
for lang in LANGS:
    vendor_queries(lang, seen)

(out_helix / "NOTICE").write_text(
    f"Files in this folder are taken from Helix {HELIX_TAG} (https://github.com/helix-editor/helix)\n"
    "and are licensed under the Mozilla Public License 2.0; see LICENSE.\n"
)

# Grammars: only <subpath>/src/ (parser.c, scanner.c, headers) plus a repo-level common/ if present.
shutil.rmtree(out_vendor, ignore_errors=True)
for lang in LANGS:
    name = languages[lang].get("grammar", lang)
    src = grammars[name]["source"]
    sub = src.get("subpath", "")
    tar = tarball(src["git"], src["rev"])
    dest = out_vendor / name
    srcdir = f"{sub}/src/" if sub else "src/"
    for rel, m in members(tar, ""):
        is_licence = rel in ("LICENSE", "LICENSE.md", "LICENSE.txt", "COPYING")
        if rel.startswith(srcdir) or rel.startswith("common/") or is_licence:
            if rel.endswith((".c", ".h", ".cc")) or is_licence:
                extract(tar, rel, m, dest / rel)  # keep repo layout: scanners include ../../common
    (dest / "REV").write_text(f"{src['git']} {src['rev']} {sub}\n")
    print("vendored", name, file=sys.stderr)
