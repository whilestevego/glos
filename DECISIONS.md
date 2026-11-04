# Decisions

Each decision made while planning glos on 2026-11-03, with revisions from a review of the plan on
2026-11-04: the options considered, what was chosen, and why. The design itself is in
[PLAN.md](PLAN.md).

## Why build it at all

- **Considered:** a Claude Code pane mod; Plannotator, Preflight or Diffsmith (browser or desktop
  apps); tuicr in a Ghostty split, which was tried and wired up to Claude Code.
- **Chosen:** a dedicated terminal tool.
- **Why:** the pane mod is capped at 45% of the terminal width in fullscreen and can't fit long
  lines. Browser and desktop tools leave the terminal. tuicr works, but has no hooks for agents,
  loses comments unless you save before quitting, and its interface isn't Helix.

## Agent integration

- **Options:** MCP server; event hooks; CLI + JSON; local socket/RPC.
- **Chosen:** MCP server and CLI + JSON.
- **Why:** MCP lets any agent use glos natively; the CLI serves scripts and agents without MCP.
- **Revised 2026-11-04:** the local socket was dropped. The shared SQLite database (see Storage)
  already carries changes between processes, and every process notices them by checking
  `PRAGMA data_version`, so replies still appear live in an open UI.

## Delivering a submitted review

- **Options:** MCP channel push; a Claude Code mod that submits a prompt; the agent polls
  (`wait_for_review`).
- **Chosen:** MCP channel push.
- **Why:** no mod is needed, and the review arrives as you submit. The other two remain as fallbacks
  (`wait_for_review` is in the tool list) pending the M0 spike.
- **Revised 2026-11-04, after M0:** `wait_for_review` is the baseline. Channels are a research
  preview: they need a launch flag and an organisation setting (blocked on the account used for
  the spike) and drop messages silently otherwise. glos supports them once they're no longer
  experimental. The Claude Code plugin can add a `Stop` hook so the agent needn't loop.

## Helix fidelity

- **Options:** keymap and modes; pickers and popups; themes and config; tree-sitter.
- **Chosen:** all four.

## Foundation

- **Options:** ratatui with Helix-style behaviour; Helix's crates as git dependencies; a fork of
  Helix.
- **Chosen:** ratatui with Helix-style behaviour.
- **Why:** stable crates.io dependencies. Helix's crates have no stable API and pull in the whole
  editor, and a fork means maintaining a large editor.
- **Revised 2026-11-04:** one exception, `helix-core`, for motions, selections and text objects.
  Rewriting them from memory is how a near-copy ends up feeling off. Pinning it to one Helix
  release avoids the unstable API, and `helix-core` alone has no UI and isn't the whole editor.
  The M0 spike checks that; the fallback is copying its files in under MPL-2.0.
- **Making the feel checkable (2026-11-04):** a generated keymap checklist (`KEYMAP.md`), tests in
  Helix's selection notation, screen snapshots of the elements that must look like Helix, and a
  side-by-side fidelity pass at the M2 checkpoint.

## Code layout (2026-11-04)

- **Options:** a Cargo workspace of eight focused crates; one crate split into modules.
- **Chosen:** one crate split into modules.
- **Why:** the crate boundaries would be guesses before any code exists. Split later if compile
  times or reuse call for it.

## Scope: review only, or editing too

- **Options:** review only; review plus light edits; review plus suggested changes.
- **Chosen:** review plus light edits, done in place with the Helix model (not by handing off to
  Helix).
- **Note:** scheduled after the MVP (see PLAN.md, Milestones).

## Storage

- **Options:** in the repo, git-ignored; in git (notes or refs); the user data dir.
- **Chosen:** the user data dir.
- **Why:** keeps repos clean. Every change is saved at once, as a direct answer to tuicr losing
  unsaved comments.
- **Format, revised 2026-11-04:** a JSON file per review with a file lock was replaced by one
  SQLite database. The UI, MCP servers and CLI all write at once; rewriting a whole JSON file lets
  one overwrite another's changes, and a socket alone needs a process that's always running to
  save anything. SQLite transactions avoid both, with no daemon.
- **Pure-Rust databases considered:** Turso (Limbo), `redb`, `fjall`, `sled`, GlueSQL. Rejected:
  none supports several processes opening the database at once, so each would need a daemon. Speed
  doesn't matter at glos's data size. `rusqlite` with `bundled` needs only a C compiler at build
  time.
- **Snapshots (2026-11-04):** what you reviewed in each round is stored in this database too, not as
  git objects, which `git gc` deletes when nothing references them and which don't exist without
  git.

## Diff views

- **Options:** unified with a split toggle; side by side with a toggle; full file with a gutter.
- **Chosen:** all three, full file + gutter by default, and switching must be easy (`space v`).

## Threads

- **Options:** resolve and reopen; outdated tracking; comment types; review-level comments.
- **Chosen:** resolve and reopen, outdated tracking, review-level comments.
- **Not chosen:** comment types (issue, nit, praise…).
- **Revised 2026-11-04:** comments go on the new side only in the MVP. A thread becomes outdated
  only once the files have been quiet for a few seconds or the round ends, and then stays outdated
  until you reopen or resolve it, so threads don't flicker while the agent edits.

## While the agent works

- **Options:** live refresh; refresh on demand; snapshot rounds.
- **Chosen:** live refresh. Rounds still exist in the data model, to support the
  "since last review" scope.

## Git library

- **Options:** gix; git2; shelling out to git.
- **Chosen:** gix. It's pure Rust and what Helix uses for its diff gutter.

## Git required or not (2026-11-04)

- **Options:** git repos only; git optional.
- **Chosen:** git optional. glos also reviews two files or two folders (`glos diff old new`). Every
  source becomes old/new text that glos diffs itself with imara-diff; gix is used only to read git
  sources. Per-commit scope is git only; since last review works everywhere.

## Review keys

- **Options:** space-mode menu; bracket motions; typable commands; selection-first comments.
- **Chosen:** all four.
- **Key help panel (2026-11-04):** essential, part of M1. As in Helix's infobox, every key that
  starts a sequence opens a panel of the keys that can follow. glos lists only the actions that
  apply where you are, and builds the panel from the live keymap so remapped keys show their new
  bindings.

## What agents can do

- **Options:** reply and resolve; pre-annotate their changes; request review; point you to a line.
- **Chosen:** all four. Pointing you to a line works only with your permission, per review.
- **Revised 2026-11-04:** pointing becomes `suggest`. It shows a notice and `space n` jumps there;
  the agent never moves your view, so no permission is needed.

## Launching

- **Options:** a standalone command; from Claude Code; when the agent asks.
- **Chosen:** all three.
- **Revised 2026-11-04:** no dependency on Ghostty or any terminal. Opening a split is an optional
  `open_command` in `config.toml` (tmux, kitty, WezTerm, or a Ghostty script). Without it, an open
  glos shows the request, or the agent asks you to run `glos`.
- **Revised 2026-11-04:** glos is the CLI and the MCP server only. Launching from Claude Code (a
  `/glos` slash command, replacing `/tuicr`) and the channel fallback mod move to a separate Claude
  Code plugin built on them.

## What a review covers

- **Options:** branch vs base; since last review; per commit; a files filter.
- **Chosen:** all four, as ways to use the same review; branch vs base is the default.

## Several agents at once

- **Options:** one review per branch; a multi-review dashboard; single for now.
- **Chosen:** a multi-review dashboard, in the MVP.

## File navigation

- **Options:** a picker plus a toggleable tree; picker only; an always-on file list.
- **Chosen:** a picker plus a toggleable tree.

## Showing threads

- **Options:** inline and collapsible; a gutter mark with a popup; a side panel.
- **Chosen:** inline and collapsible, like Helix's inline diagnostics.

## Mouse

- **Options:** full; basic; none.
- **Chosen:** full: click, drag to select, scroll, and click files, threads and popup buttons.

## Platforms

- **Options:** macOS and Linux; macOS only; all three.
- **Chosen:** macOS and Linux.

## Grammars and themes

- **Options:** reuse Helix's runtime; bundle common grammars; both.
- **First chosen:** reuse Helix's runtime at run time. Exactly your editor's colours; without
  Helix, plain text.
- **Revised 2026-11-04:** bundle instead. Everything taken from Helix (grammars, queries, themes,
  `languages.toml`) is compiled into glos from a pinned Helix release, so glos runs the same
  whether Helix is installed or not, and a Helix upgrade can't break it. glos no longer reads
  `~/.config/helix`; your own themes go in `~/.config/glos/themes/`.

## MVP

- **Options:** reading and commenting; threads both ways; all three views; the dashboard.
- **Chosen:** all four. In-place editing follows the MVP.
- **Checkpoint (2026-11-04):** after M2, glos is used for real reviews instead of tuicr before M3–M5
  are built, and the plan is adjusted from that.

## UX review (2026-11-04)

A review of the plan for terminal UX and CLI conventions led to these decisions, all now in
PLAN.md:

- **Writing comments:** a small Helix-style box (insert mode first, `esc` then `enter` saves),
  drafts saved as you type, `:editor` for `$EDITOR`. Your comments can be edited and deleted;
  after a submit, edits are marked.
- **Submitting:** every submit carries a verdict (approve, changes requested, comment). The count
  of unsubmitted comments is always visible.
- **Unread replies** are tracked per thread (`]u`).
- **Any file** in the repo can be opened read-only (`space f`) for context.
- **A review's life:** open, closed (`:close`) or archived (automatically when the branch is merged
  or deleted); `glos gc` cleans up.
- **Keys follow Helix's concepts:** changes are hunks (`]g`, `space g`), threads are diagnostics
  (`]d`, `space d`), changed files are like buffers (`gn`, `gp`). This replaces `]h`, `]t`, `]f`,
  `]c` and the `space f` changed-file picker, which contradicted Helix. Commits move to `]C`, the
  dashboard to `space l`.
- **Mouse** can be turned off, and `space y` yanks to the clipboard (OSC 52 over SSH).
- **The terminal:** control characters are stripped from everything drawn; logs go to a file,
  never to stdout or the screen; panic restore, `ctrl-z`, `NO_COLOR` and a 256-colour fallback;
  soft wrap off by default with horizontal scrolling; live refresh never moves your view.
- **Kinds of change:** binary, renamed, deleted, mode and symlink changes are shown; generated
  files and lockfiles start collapsed. The first screen draws before the whole diff is computed.
- **CLI:** `glos diff A B` for files and folders, so paths can't collide with subcommands; flat
  subcommands (`glos list`, not `glos review list`); readable output by default and versioned
  `--json`; documented exit codes; `glos doctor`; completions and a man page.
- **Paths:** XDG folders on macOS too, as Helix does, plus `--db` / `GLOS_DB`. The schema is
  versioned with `PRAGMA user_version`, with a backup before each migration.
- **Agents:** `get_review` returns a summary plus what's new, with full threads on request.
  Sessions are identified by the MCP server's process id and the client's `clientInfo` name.

## Name

- **First pick:** loupe (a jeweller's magnifier), dropped because it clashes with GNOME's image
  viewer (its command is `loupe` on many Linux desktops) and an existing crate.
- **Then:** about 100 names were checked against crates.io, Homebrew, Linux distro packages
  (Repology), commands on the PATH and GitHub. Long names (kibitzer, obelus, margent, scholion)
  were rejected as too long; most short words (stet, gloss, lgtm, ptal, nits, quirk) were taken.
- **Chosen:** glos, from *gloss*: a note written in the margin of a text. Four letters; free on
  crates.io, Homebrew and Linux distros, with no notable GitHub project of that name (checked
  2026-11-03). Trademarks and domains were not checked.

## M0 spikes (2026-11-04)

- **Helix release:** pinned to 25.07.1, the newest.
- **Highlighting:** `tree-house` 0.3.0, not `tree-sitter-highlight`. It's the version helix-core
  already links, so there's one tree-sitter runtime, and it takes statically linked grammars.
- **Vendored grammars:** committed, not fetched at build time. Costs about 5 MB of git history per
  Helix upgrade; builds never need the network.
- **helix-core:** kept as a git dependency, not copied in. It brings only helix-loader,
  helix-stdx and helix-parsec. The keymap wrappers from helix-term are ported into one MPL file.
- **gix:** one `status()` pass with the merge-base as its head tree covers committed, staged,
  unstaged and untracked changes and renames. No fallback to the git command needed.
- **Channels:** declared through rmcp's experimental capabilities; the protocol is capped at
  2025-11-25 so pushes can be delivered. Needs a launch flag while channels are in preview.

## The plan folder

- **Chosen:** PLAN.md, this decision log, and a git repository. No Cargo skeleton yet.
