# glos

glos is a code reviewer for your terminal, made for reviewing work done by coding agents such as
Claude Code.

When an agent finishes a task, you usually want to read what it changed and tell it what to fix.
Doing that on GitHub means pushing a branch, opening a pull request and switching to a browser,
for code you may not want to publish yet. glos lets you do it where you already are. You read
the changes, leave comments on the lines you care about, and send them to the agent. The agent
replies on the same lines, fixes things, and asks you to look again. The review stays open until
you're happy with it.

If you use [Helix](https://helix-editor.com), glos will feel familiar: it has the same keys,
the same modes, the same themes and the same syntax highlighting.

## What it can review

- **A git branch.** Run `glos` inside a repository. It shows everything your branch changed since
  it left `main`, including uncommitted and untracked files.
- **Git revisions.** `glos diff HEAD~1` compares a commit with your files as they are now, and
  `glos diff HEAD~3 HEAD~1` compares two commits.
- **Two files or two folders.** `glos diff old.rs new.rs` or `glos diff old/ new/`. No git
  needed.

## Installing

glos is written in Rust. You need [Rust](https://rustup.rs) and a C compiler (the syntax
highlighting grammars are compiled into glos). Then run:

```sh
cargo install --git https://github.com/whilestevego/glos
```

It works on macOS and Linux. Run `glos doctor` afterwards to check that everything is set up.

## Connecting your agent

glos talks to agents through MCP. To set it up with Claude Code, run this once:

```sh
claude mcp add glos -- glos mcp
```

After that, your agent can ask you for a review when it's done, wait for your comments, reply to
them, and mark threads as resolved. It can also leave its own notes on its changes before you
start ("I wasn't sure about this part"), and point you at a line it wants you to look at.

Agents without MCP can do the same things from the command line: `glos reply`, `glos resolve`,
`glos annotate`, `glos show` and so on. Add `--json` to any of them for output a program can read.

## A review, start to finish

1. The agent finishes its work and asks for a review. If glos is already open, you'll see the
   request at the bottom of the screen. If not, run `glos` in the project folder.
2. Read through the changes. Changed lines are marked in the gutter. `]g` jumps to the next
   change and `gn` to the next file. The file tree on the left shows every changed file.
3. To comment, put the cursor on a line (or select a few with `x`) and press `space c`. Type your
   comment, press `esc`, then `enter` to save it. You can't lose a comment: everything is saved
   as you type, even if you quit or glos crashes.
4. When you're done, press `space s` and pick a verdict: approve, request changes, or just send
   comments. The agent gets all your comments at once.
5. As the agent works, glos updates by itself without moving your screen. Its replies show up
   under your comments, marked unread. Press `]u` to jump to the next one.
6. For the next round, press `space o` then `r` to see only what changed since your last review.
   Reply, resolve or reopen threads, and submit again.
7. Once you've approved, `:close` closes the review.

If your code changes under a comment, glos moves the comment along with the code. If the code
it was about is gone, the comment is marked outdated and shows the code as it was.

## Keys

glos uses Helix's keys for moving around and selecting: `hjkl`, `w`, `b`, `gg`, `x`, `v`, `/`
and the rest. On top of those, it adds keys for reviewing:

| Key | What it does |
|---|---|
| `space c` | Comment on the selected lines |
| `space C` | Comment on the review as a whole |
| `space r` | Reply to the comment under the cursor |
| `space m` | Edit your latest comment |
| `space x` | Resolve or reopen a thread |
| `space s` | Submit the review |
| `space v` | Change the view: whole file, unified, or side by side |
| `space o` | Change what's shown: the whole branch, since your last review, one commit at a time, or a filter |
| `space g` | Pick a changed file |
| `space f` | Open any file in the project, for context |
| `space d` | Pick a thread in this file (`space D` for the whole review) |
| `space e` | Show or focus the file tree |
| `space l` | The dashboard: every open review |
| `space n` | Go to the line the agent asked you to look at |
| `]d` `[d` | Next or previous thread |
| `]u` `[u` | Next or previous unread reply |
| `gn` `gp` | Next or previous changed file |
| `enter` | Open or collapse the thread under the cursor |

You don't need to remember these. Press `space` (or `g`, `]`, and so on) and wait, and a panel
lists the keys that can follow. Press **`Ctrl-G` anywhere** to see every key that works where you
are, whether that's the file view, the comment box, the file tree or a list.

The mouse works too. Click to move, drag to select, and click a thread to open it.

## Several agents at once

If you have agents working on several branches or worktrees, `glos dashboard` (or `space l`
inside glos) lists every open review. Each row shows what the agent is doing and what's waiting
on you, like unsent comments and unread replies. Press `enter` to open a review.

## Settings

Settings go in `~/.config/glos/config.toml`:

```toml
# Any Helix theme. glos comes with all of them.
theme = "catppuccin_mocha"

# Optional: how glos opens itself in a new pane when an agent asks for a review.
# Give the start of the command; glos adds the rest.
open-command = "tmux split-window -h"

[editor]
mouse = true          # set to false to turn the mouse off
file-tree = true      # show the file tree when glos starts

[editor.soft-wrap]
enable = false        # wrap long lines
```

For kitty, use `open-command = "kitten @ launch --location=vsplit"`. For WezTerm, use
`open-command = "wezterm cli split-pane --"`. Without an `open-command`, glos never opens windows
by itself, and the agent will ask you to run `glos` instead.

You can put your own themes in `~/.config/glos/themes/`, in Helix's format.

## Other commands

| Command | What it does |
|---|---|
| `glos list` | List open reviews (`--all` includes closed ones) |
| `glos show` | Print a review's comments and replies |
| `glos open` | Open glos on this folder's review, using your `open-command` |
| `glos close` | Close a review |
| `glos gc` | Clean out old, finished reviews |
| `glos completions zsh` | Shell completions (also `bash`, `fish` and others) |
| `glos doctor` | Check your setup |

## Where your data lives

Everything stays on your machine. Reviews are kept in a single database at
`~/.local/share/glos/glos.db`, and logs go to `~/.local/state/glos/glos.log`. Nothing is ever
sent anywhere except to the agents you've connected.

Reviews close themselves once their branch is merged or deleted. `glos gc` removes reviews that
have been finished for more than 30 days.

## Credits

glos borrows a lot from Helix: its editing core, its themes, its syntax queries and its key
layout. The files taken from Helix are in the `helix/` folder and stay under Helix's licence,
MPL-2.0.
