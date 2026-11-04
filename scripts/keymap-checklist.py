#!/usr/bin/env python3
"""Generate KEYMAP.md: every key in Helix's default normal-mode keymap at the pinned release,
and what glos does with it (done, replaced, planned, not applicable, or to do).

Reads Helix's helix-term/src/keymap/default.rs (downloaded at HELIX_TAG, or a path as the first
argument) and glos's src/ui/keymap.rs. Rerun after changing the keymap or moving Helix release.
"""
import re, sys, urllib.request
from pathlib import Path

HELIX_TAG = "25.07.1"
root = Path(__file__).resolve().parent.parent

if len(sys.argv) > 1:
    helix_src = Path(sys.argv[1]).read_text()
else:
    url = f"https://raw.githubusercontent.com/helix-editor/helix/{HELIX_TAG}/helix-term/src/keymap/default.rs"
    helix_src = urllib.request.urlopen(url).read().decode()

# Only the normal-mode map: select mode is the same keys extending, insert mode needs editing.
helix_src = helix_src[helix_src.index('let normal = keymap!'):helix_src.index('let mut select')]


def parse_helix(src):
    """(key path, command) pairs from the keymap! macro."""
    out, stack = [], []
    for line in src.splitlines():
        line = line.split("//")[0].strip()
        if m := re.match(r'^((?:"[^"]+"\s*\|\s*)*"[^"]+")\s*=>\s*\{\s*"([^"]+)"', line):
            stack.append(re.findall(r'"([^"]+)"', m.group(1))[0])
        elif m := re.match(r'^((?:"[^"]+"\s*\|\s*)*"[^"]+")\s*=>\s*([a-z_]+)', line):
            for key in re.findall(r'"([^"]+)"', m.group(1)):
                out.append((" ".join(stack + [key]), m.group(2)))
        elif line.startswith("}") and stack:
            stack.pop()
    return out


def parse_glos(src):
    """(key path, command) pairs from normal_defs() in src/ui/keymap.rs, with helper menus like
    view_defs() expanded."""
    def defs_of(fn):
        start = src.index(f"fn {fn}()")
        return src[start:src.index("\n}\n", start)]

    def keys(spec):
        return ["|"] if spec == "|" else spec.split("|")

    def walk(body, stack, out):
        for line in body.splitlines():
            line = line.strip()
            if m := re.match(r'(?:sticky_)?menu\("([^"]+)",\s*"[^"]+",\s*([a-z_]+)\(\)\)', line):
                walk(defs_of(m.group(2)), stack + [m.group(1)], out)
            elif m := re.match(r'(?:sticky_)?menu\("([^"]+)",\s*"[^"]+",\s*vec!\[', line):
                stack = stack + [m.group(1)]
            elif m := re.match(r'cmd\("([^"]+)",\s*"([a-z_]+)"\)', line):
                for key in keys(m.group(1)):
                    out.append((" ".join(stack + [key]), m.group(2)))
            elif line.startswith("]") and stack:
                stack = stack[:-1]
        return out

    return walk(defs_of("normal_defs"), [], [])


# Keys PLAN.md gives to review actions, and when they arrive.
PLANNED = {
    "space c": "comment on the selection (M2)", "space C": "review-level comment (M2)",
    "space r": "reply to the thread (M3)", "space m": "edit your latest comment (M2)",
    "space x": "resolve or reopen the thread (M3)", "space s": "submit with a verdict (M2)",
    "space v": "switch view (M4)", "space o": "switch scope (M4)",
    "space d": "thread picker, this file (M2)", "space D": "thread picker, whole review (M2)",
    "space e": "file tree (M4)", "space l": "dashboard (M5)", "space n": "jump to the agent's suggestion (M3)",
    "]d": "next thread (M2)", "[d": "previous thread (M2)",
    "]D": "last thread (M2)", "[D": "first thread (M2)",
    "]u": "next unread reply (M3)", "[u": "previous unread reply (M3)",
    "]C": "next commit (M4)", "[C": "previous commit (M4)",
}

NOT_APPLICABLE = [
    ("editing comes after the MVP", r"^(insert|append|open_|delete|change|replace|paste|undo|redo|earlier|later|indent|unindent|format|join|switch_case|switch_to_|toggle_.*comments|add_newline|surround_(add|replace|delete)|increment|decrement|commit_undo|smart_tab|completion|kill_|dot_repeat|replace_selections_with|yank_joined|reflow|align_selections|rotate_selection_contents)"),
    ("needs a language server", r"^(goto_(definition|declaration|type_definition|reference|implementation)|hover|rename_symbol|code_action|symbol_picker|workspace_symbol_picker|diagnostics_picker|workspace_diagnostics_picker|goto_(next|prev|first|last)_diag|select_references_to_symbol_under_cursor|signature_help|workspace_command_picker)$"),
    ("glos has no buffers or windows", r"^(buffer_picker|goto_last_accessed_file|goto_last_modified_file|goto_last_modification|rotate_view|hsplit|vsplit|transpose_view|goto_file_[hv]split|wclose|wonly|jump_view_|swap_view_|hsplit_new|vsplit_new|goto_file|file_picker_in_current_directory|file_explorer_in_current_buffer_directory|last_picker)"),
    ("no debugger", r"^dap_"),
    ("no shell commands on a read-only review", r"^(shell_|pipe|keep_pipe)"),
]

helix = parse_helix(helix_src)
glos = dict(parse_glos((root / "src/ui/keymap.rs").read_text()))
rows, counts = [], {}
for key, cmd in helix:
    g = glos.get(key)
    if g == cmd:
        status = "done"
    elif key in PLANNED:
        status = f"replaced: {PLANNED[key]}"
    elif g:
        status = f"replaced: `{g}`"
    elif reason := next((r for r, pat in NOT_APPLICABLE if re.search(pat, cmd)), None):
        status = f"not applicable: {reason}"
    else:
        status = "to do"
    counts[status.split(":")[0]] = counts.get(status.split(":")[0], 0) + 1
    rows.append(f"| `{key.replace("|", "\\|")}` | `{cmd}` | {status} |")

helix_keys = {k for k, _ in helix}
extra = [f"| `{k}` | `{c}` |" for k, c in glos.items() if k not in helix_keys]
planned_new = [f"| `{k}` | {v} |" for k, v in PLANNED.items() if k not in helix_keys and k not in glos]

out = [
    "# Keymap checklist",
    "",
    f"Generated by `scripts/keymap-checklist.py` from Helix {HELIX_TAG}'s default normal-mode keymap",
    "and glos's `src/ui/keymap.rs`. Select mode is the same keys extending the selection; insert mode",
    "comes with editing. Don't edit by hand: rerun the script.",
    "",
    "Summary: " + ", ".join(f"{n} {s}" for s, n in sorted(counts.items(), key=lambda x: -x[1])) + ".",
    "",
    "| Key | Helix command | glos |",
    "|---|---|---|",
    *rows,
    "",
    "## Keys glos adds",
    "",
    "| Key | glos command |",
    "|---|---|",
    *extra,
    *([""] + ["Planned, on keys Helix leaves free:", "", "| Key | Action |", "|---|---|"] + planned_new if planned_new else []),
    "",
]
(root / "KEYMAP.md").write_text("\n".join(out))
print("wrote KEYMAP.md:", ", ".join(f"{n} {s}" for s, n in counts.items()))
