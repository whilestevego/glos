# Components: splitting `App` along the ratatui Component pattern

`src/ui/mod.rs` is about 3,400 lines, and nearly all of it is one `App` with about 50 fields and 60
methods. Input goes through a hand-ordered if-chain in `App::key`, rendering goes through a
hand-ordered list in `App::render`, and the mouse goes through a third chain in `App::mouse`. Each
new overlay or pane means editing all three. This plan gives the self-contained parts of the UI one
shared interface. It moves them out of `mod.rs` and replaces the if-chains with routing by focus.
It doesn't change what any key, click or screen does.

## Where we are

Some of these parts are already components in all but name:

| Part | State lives in | Keys | Render | Mouse |
|---|---|---|---|---|
| `Picker` | `picker.rs` | `Picker::key -> Outcome` | `Picker::render` + `App::render_picker` (preview) | `App::mouse` |
| Palette | `App.palette: Option<Option<Picker>>` | via picker | via picker | via picker |
| `Prompt` | `App.prompt` (struct in `mod.rs`) | `App::prompt_key`, `App::search_preview` | `App::render_command_line`, `App::render_completion` | swallowed |
| `Tree` | `tree.rs`, `focused` flag | `App::tree_key` | `Tree::render` | `App::mouse` |
| `CommentBox` | `comments.rs` | `App::box_key`, `CommentBox::insert_key` | `App::render_box` | swallowed |

Today's key order (`App::key`): Ctrl-G palette toggle → picker → prompt → focused tree → `next_key`
→ comment box (only when no keys are pending) → count → keymap. The refactor has to keep this order
exactly.

## The design

A new file, `src/ui/component.rs`:

```rust
/// What a component did with an event.
pub enum Outcome {
    /// Not mine: the next layer gets it.
    Ignored,
    /// Handled; nothing for App to do.
    Consumed,
    /// Handled, and App has to act on it.
    Msg(Msg),
}

/// What a component asks App to do. App owns the review, the store and the files; components
/// never see `&mut App`.
pub enum Msg { /* grows step by step; see below */ }

/// What every component may read.
pub struct Cx {
    pub screen: Rect,
}

pub trait Component {
    fn key(&mut self, key: Key, cx: &Cx) -> Outcome;
    fn mouse(&mut self, _m: MouseEvent, _area: Rect, _cx: &Cx) -> Outcome {
        Outcome::Ignored
    }
}
```

### As built

The implementation departs from the first draft of this plan in three ways:

- **No `render` on the trait, and `Cx` holds only the screen.** Each part needs different things
  from App to draw. The picker's preview needs App's files, and the box needs the loader and the
  selection colours for App's mode. So each part has its own `render` with the arguments it needs,
  and `App::render` keeps the fixed z-order. With `Cx` reduced to the screen size, `glos dashboard`
  (which has no `Loader`) can use the same trait.
- **The tree keeps its `focused` flag.** There's one pane, so a `Focus` enum would just move the
  flag.
- **The box no longer parses its own text.** `CommentBox::insert_key` returns whether the text
  changed, and App re-parses it when it gets `Msg::BoxEdited`. That keeps the `Loader` out of `Cx`.

Decisions:

- **Concrete fields, not `Box<dyn Component>`.** App keeps `pickers: Vec<Picker>`,
  `prompt: Option<Prompt>`, `tree: Option<Tree>` and `comment_box: Option<CommentBox>`. App also
  reaches into them directly: it renders the picker's preview from its own files, and it edits the
  box's `file` through `file_mut`. A trait object would force downcasting for both. The trait gives
  every component the same contract, and the routing code calls it on concrete types.
- **Components return `Msg`; App applies it.** Most `Msg` arms just call the App method that
  exists today (`open`, `typed_command`, `close_box`, `save_box`…). Cross-cutting rules such as
  `blocked_in_box` stay in App, since they are app policy and not part of any component.
- **`Ignored` falls through.** This matters for the comment box. In normal mode, any key it doesn't
  own (`ret`, `q`, `i`, `a`) goes on to the editor keymap, which edits the box's text. That
  behaviour stays as it is.
- **The editor core stays in App.** That covers modes, the keymap trie, counts, `next_key`, macros,
  jumps, the `File` views, alt and split docs, threads and the statusline. Nearly every method
  there reads both the files and the threads, so turning it into a component would just pass most
  of App through `Cx`. It's out of scope here; we can revisit it later if it starts to hurt.

## Steps

Each step is one commit on a `components` branch. Each commit passes `cargo fmt`, `cargo clippy`
and `cargo test` with **no snapshot changes**. `src/ui/tests.rs` drives everything through
`App::key` and `App::mouse`, so it checks that behaviour hasn't changed. Each step also adds unit
tests for the component on its own: keys in, `Outcome` out.

### 1. The trait, on `Picker`

- Add `component.rs` with `Outcome`, `Msg { Open(Target) }`, `Cx` and the trait.
- Implement `Component for Picker`. Map today's `picker::Outcome::{Stay, Close, Open}` onto
  `Consumed`, a new `Msg::Close`, and `Msg::Open`, then delete `picker::Outcome`. Move the page
  calculation (`page(areas(screen).0)`) inside `Picker::key`, using `cx.screen`.
- Move the picker's scroll and click handling from `App::mouse` into `Picker::mouse`.
- `Picker::render` draws the list and the preview frame. App still fills the preview from
  `selected()`, as it does in `render_picker` today.
- Add `App::apply(msg) -> Option<AppAction>`. It takes over the body of `App::choose`.

Done when `App::key` and `App::mouse` reach the picker only through the trait.

### 2. A picker stack replaces `picker` and `palette`

- Replace `picker: Option<Picker>` and `palette: Option<Option<Picker>>` with `pickers: Vec<Picker>`.
- Ctrl-G with the palette on top pops it. Otherwise it pushes a new palette. The palette needs a
  marker so App knows which picker it is: either a `palette: bool` on `Picker`, or the existing
  `dashboard::is_dashboard`-style check.
- `Msg::Close` pops. `Target::Nothing` pops the palette and leaves the picker under it showing.
  `Msg::Open` clears the stack and then opens the target.

Done when `palette` is gone and the palette-over-picker tests pass unchanged.

### 3. `Prompt` becomes a component

- Move `Prompt`, `PromptKind` and `completions()` into `src/ui/prompt.rs`.
- Move `prompt_key`'s line editing and tab completion into `Prompt::key`. Its effects on the file
  become messages:
  - `Msg::Preview { kind, line }` on every edit. App runs today's `search_preview` from it.
  - `Msg::Cancel { kind }` on `esc`, ctrl-c, or backspace on an empty line. App puts back `origin`
    and `top`.
  - `Msg::Submit { kind, line }` on `ret`. App runs today's submit arms (`typed_command`, regex
    apply, search plus `last_search`).
- Move `render_command_line`'s prompt branch and `render_completion` into `Prompt::render`. The
  statusline-message branch stays in App.
- Clicks while the prompt is open stay swallowed: `Prompt::mouse` returns `Consumed`.

### 4. `Tree` becomes a component

- Replace `tree.focused` with `App.focus: Focus { Editor, Tree }`. Find every place that reads or
  writes `focused` and switch it over.
- Move `tree_key` into `Tree::key`. `j`/`k` are `Consumed`; `ret`/`l` sends `Msg::Open(Path)` and
  App sets focus back to the editor; `esc` sends `Msg::Unfocus`; `q` sends `Msg::CloseTree`.
- Move the tree's click handling into `Tree::mouse`, which returns `Msg::Open(Path)`.
- Pass the current path to `Tree::render`, either through `Cx` or as a field App sets before
  drawing.

### 5. `CommentBox` becomes a component

- Move `box_key` into `CommentBox::key`. Insert mode is `Consumed`; App calls `save_draft` and
  `scroll_to_cursor` whenever the text changed (`Msg::Edited`). In normal mode, `ret` sends
  `Msg::SaveBox`, `q` sends `Msg::CloseBox`, and `i`/`a` are `Consumed` (App resets `mode`). Every
  other key is `Ignored`, so it reaches the editor keymap exactly as it does today.
- Move `render_box` into `CommentBox::render`.
- Clicks while the box is open stay swallowed.
- Leave the other `comment_box` references in App (`file_mut`, layout, selection styles,
  `blocked_in_box`, `:editor`) as they are. They are the editor reading the box, not the box
  handling input.

### 6. Routing

With every part behind the trait, rewrite the three chains:

```rust
fn key(&mut self, key: Key) -> Option<AppAction> {
    // recording, status reset, Ctrl-G: unchanged
    let cx = self.cx();
    let outcome = if let Some(p) = self.pickers.last_mut() { p.key(key, &cx) }
        else if let Some(p) = &mut self.prompt { p.key(key, &cx) }
        else if self.focus == Focus::Tree { self.tree.as_mut().map_or(Outcome::Ignored, |t| t.key(key, &cx)) }
        else { Outcome::Ignored };
    // then next_key, the box (when nothing's pending), count, keymap: unchanged
}
```

`render` and `mouse` follow the same order. The editor's own key handling (`next_key`, count,
keymap) stays inline in `key` and is not a component.

Done when `mod.rs` has no `prompt_key`, `tree_key`, `box_key`, `render_box`,
`render_command_line` prompt branch, `render_completion`, or picker-specific mouse code.

## Risks

- **`Cx` borrowing.** `Cx` borrows `theme` and `loader` from App while a component field is
  borrowed mutably. These are disjoint fields, so this compiles as long as `cx()` builds `Cx` from
  the fields directly and not through a `&self` method. If that turns out awkward, build `Cx` inline
  at each call site.
- **Order regressions.** The box only takes keys when `pending` is empty, and `next_key` comes
  before it. Step 6 must keep both. The existing tests for keys in the box and menus pressed inside
  the box cover this. If a reorder slips through without a test catching it, add one.
- **Scope creep into the editor.** If a step starts needing `files`, `threads` or `alt` in `Cx`,
  stop: that logic belongs in App behind a `Msg`.

## Out of scope

- Turning the editor (file view, modes, keymap, statusline, help panel) into a component.
- Changing any behaviour, key or look. Snapshots are the contract.
- A generic layout or focus system beyond `Focus { Editor, Tree }`.
