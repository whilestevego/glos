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
them, and mark threads as resolved. glos never guesses what the agent wants reviewed: the agent
says exactly which branch (and which base), or which two files or folders, and gets back a review
id that it uses from then on. It can also leave its own notes on its changes before you
start ("I wasn't sure about this part"), and point you at a line it wants you to look at.

Agents without MCP can do the same things from the command line. `glos open-review git . topic
main` (or `glos open-review diff old new`) prints a review's id, and `glos reply`, `glos resolve`,
`glos annotate`, `glos show` and the rest take it as `--review <id>`. Add `--json` to any of them
for output a program can read.

## A review, start to finish

1. The agent finishes its work and asks for a review. If glos is already open, you'll see the
   request at the bottom of the screen. If not, run `glos` in the project folder.
2. Read through the changes. Unchanged code is folded away, leaving three lines around each
   change. Changed lines are marked in the gutter. `]g` jumps to the next change and `gn` to the
   next file. The file tree on the left shows every changed file.
3. To comment, put the cursor on a line (or select a few with `x`) and press `' c`. Type your
   comment, press `esc`, then `enter` to save it. You can't lose a comment: everything is saved
   as you type, even if you quit or glos crashes.
4. When you're done, press `' s` and pick a verdict: approve, request changes, or just send
   comments. The agent gets all your comments at once.
5. As the agent works, glos updates by itself without moving your screen. Its replies show up
   under your comments, marked unread. Press `]u` to jump to the next one.
6. For the next round, press `' o` then `r` to see only what changed since your last review.
   Reply, resolve or reopen threads, and submit again.
7. Once you've approved, `:close` closes the review.

If your code changes under a comment, glos moves the comment along with the code. If the code
it was about is gone, the comment is marked outdated and shows the code as it was.

## Keys

glos uses Helix's keys for moving around and selecting: `hjkl`, `w`, `b`, `gg`, `x`, `v`, `/`
and the rest. On top of those, it adds keys for reviewing:

| Key | What it does |
|---|---|
| `' c` | Comment on the selected lines |
| `' C` | Comment on the review as a whole |
| `' r` | Reply to the comment under the cursor |
| `' m` | Edit your latest comment |
| `' M` | Delete your latest comment, if it isn't sent yet |
| `' x` | Resolve or reopen a thread |
| `' s` | Submit the review |
| `' v` | Change the view: whole file, unified, or side by side |
| `' o` | Change what's shown: the whole branch, since your last review, one commit at a time, or a filter |
| `' d` | Pick a thread in this file (`' D` for the whole review) |
| `' l` | The dashboard: every open review |
| `' n` | Go to the line the agent asked you to look at |
| `space g` | Pick a changed file |
| `space f` | Open any file in the project, for context |
| `space e` | Open, focus or close the file tree |
| `]r` `[r` | Next or previous thread (`]R` `[R` for the last and first) |
| `]u` `[u` | Next or previous unread reply |
| `gn` `gp` | Next or previous changed file |
| `enter` | Open or collapse the thread under the cursor, or open a fold |

Review commands live under `'` so that Helix keeps its own keys. Until glos has language
servers and editing, `space c`, `space C`, `space r`, `space s`, `space d`, `space D` and `]d`
still work as before.

### All files in one scroll

`' v a` switches between showing one file at a time and showing every changed file in one
scroll. Each file starts with a rule across the screen that names its path, its added and
removed lines, and its open threads. While you scroll through a file, its header stays pinned
to the top of the screen. `:all` and `:one` also switch, and `all-files = true` in the settings
starts glos in one scroll.

Click a file's header to collapse the file, and click it again to expand it. A collapsed file
shows a line such as `↕ 42 lines` under its header in place of its code, and a click
on that line expands it too.
`enter` does the same for the file the cursor is in, unless the cursor is on a fold or a thread.
A collapsed file keeps its folds, so it opens again as you left it.

In one scroll, `j` and `k` carry on into the next and previous file, `gg` and `G` go to the
start of the first file and the end of the last, and the page keys scroll through the whole
review. `gn`, `gp`, the tree and `space g` go to a file's header. The other files are read only
when they come near the screen, so a big review opens as fast as one file. For now this works in
the whole-file view, and the unified and side-by-side views show one file. The mouse wheel
scrolls through every file, and a click in a file's code moves the cursor there. Search, `]g`
and the thread keys carry on into the next file when there's nothing more in this one, and `n`
goes round from the last file to the first.

### Folding

Lines far from any change are folded into a band across the screen that says which lines it
hides, like `↕ lines 16–31 (16)  ret opens`. Lines with a comment on them are never folded. Press `enter` on a fold, or click it, to open it. The rest of
the folding keys are under `z`, as in Vim:

| Key | What it does |
|---|---|
| `zo` | Open the fold under the cursor |
| `za` | Open or close the fold under the cursor |
| `zR` | Open every fold in the file |
| `zM` | Fold everything again |
| `z+` `z-` | Show one more or one fewer line around each change |
| `zf` | Fold the selected lines yourself |
| `zd` | Remove a fold you made |

A fold you make can hide anything, even changes and comments, and it says what it's hiding:
`↕ lines 40–51 (12) · 3 changed · 1 thread`. Jumping to something inside it (`]g`, `]r`, a picker)
opens it again.

To fold a whole function, select it with `maf` and press `zf`. glos has Helix's syntax-aware
selections: `maf` and `mif` for a function, `mat` for a type, `mac` for a comment, `maa` for an
argument, and `]f`, `[f`, `]t` and so on to jump between them.

You don't need to remember these. Press `space` (or `g`, `]`, and so on) and wait, and a panel
lists the keys that can follow. Press **`Ctrl-G` anywhere** to see every key that works where you
are, whether that's the file view, the comment box, the file tree or a list. Its top border
names the context those keys belong to, such as `normal`, `tree` or `box_insert`.

The mouse works too. Click to move the cursor, drag to select, and click a thread to open it.
Clicking the file tree or the code moves keyboard focus there.
In the tree, `ret` or a click opens and closes a folder, `h` and `l` close and open folders as in
Helix's file explorer, and `/` searches the names. The scroll wheel scrolls whatever
is under the pointer, including the tree, the comment box, the key menu and the picker's preview.
When lines aren't wrapped, you can scroll the code sideways with a touchpad swipe or by holding
Shift while you turn the wheel.

## Several agents at once

If you have agents working on several branches or worktrees, `glos dashboard` (or `' l`
inside glos) lists every open review. Each row shows what the agent is doing and what's waiting
on you, like unsent comments and unread replies. Press `enter` to open a review.

## Settings

Settings go in `~/.config/glos/config.toml`:

```toml
# Any Helix theme. glos comes with all of them, and uses catppuccin_mocha unless you
# choose another. "default" is Helix's own default theme.
theme = "catppuccin_mocha"

# Optional: how glos opens itself in a new pane when an agent asks for a review.
# Give the start of the command; glos adds the rest.
open-command = "tmux split-window -h"

[editor]
mouse = true          # set to false to turn the mouse off
file-tree = true      # show the file tree when glos starts
diff-context = 3      # unchanged lines shown around each change; the rest are folded
all-files = false     # show every changed file in one scroll, rather than one at a time

[editor.soft-wrap]
enable = false        # wrap long lines
```

For kitty, use `open-command = "kitten @ launch --location=vsplit"`. For WezTerm, use
`open-command = "wezterm cli split-pane --"`. Without an `open-command`, glos never opens windows
by itself, and the agent will ask you to run `glos` instead.

You can put your own themes in `~/.config/glos/themes/`, in Helix's format. Folds use
`ui.virtual.fold` if your theme sets it, and the current-line colour otherwise.

Changed lines get a tinted background, and the words that changed within them get a stronger
tint. Helix themes have no keys for these, so glos works them out from your theme. It mixes
`ui.background` toward `diff.plus` or `diff.minus`. A theme can set its own with four keys that
Helix ignores:

| Key | Used for |
|---|---|
| `diff.plus.line` | Added and modified lines |
| `diff.minus.line` | Removed lines |
| `diff.plus.word` | Changed words in an added line |
| `diff.minus.word` | Changed words in a removed line |

A theme that leaves `ui.background` unset uses the terminal's own background, which glos can't
know. Changed lines in such a theme are marked only in the gutter, unless it sets these keys.

## Other commands

| Command | What it does |
|---|---|
| `glos list` | List open reviews (`--all` includes closed ones) |
| `glos show --review <id>` | Print a review's comments and replies |
| `glos open --review <id>` | Open glos on a review, using your `open-command` |
| `glos close --review <id>` | Close a review |
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
layout. Everything taken from Helix lives in its own crate, `crates/glos-helix`, which stays under
Helix's licence, MPL-2.0.
