# Mouse: focus follows clicks, the wheel scrolls what's under it

**Status:** Implemented (2026-11-06) in PR #5.

glos sends a mouse event to whichever layer is on top, whatever the pointer is over. This has
three effects:

- A click doesn't move focus.
- The wheel scrolls only the code.
- Nothing scrolls sideways.

This plan sends each mouse event to the area under the pointer, so clicking focuses what you
clicked and anything that can't show all of its content scrolls.

Decided:

- A click focuses what it lands on.
- The wheel scrolls what's under the pointer, not what has focus.
- A sideways swipe scrolls the code horizontally.

## Where we are

`App::mouse` (`src/ui/mod.rs`) asks the layers in this order, and the first one that isn't
`Ignored` wins:

1. **An open picker** gets every event. The wheel moves its selection, and a click opens the row
   under it.
2. **The command line**, then **the comment box**, swallow every event.
3. **The tree** gets an event when it's left of `tree_area.right()`. A click on a file opens it
   and *unfocuses* the tree. A click on a folder, the title or empty space does nothing, and the
   wheel does nothing.
4. **A thread row**: a click toggles the thread.
5. **The code**: the wheel scrolls 3 lines, a click places the cursor and a drag selects.

Bugs and gaps:

- **Clicking the code while the tree has focus** moves the cursor, but `tree.focused` stays true,
  so the next keys still go to the tree.
- **The tree's view can't scroll on its own.** `Tree::render` puts `top` wherever the cursor is
  on every draw.
- **Other places that can overflow don't scroll:**
  - the comment box: a long comment
  - the key help panel: a long menu on a short terminal
  - the `:` completion list: at most 10 rows
  - the picker preview: the wheel over it moves the list's selection instead
- **Sideways scrolling:** `MouseEventKind::ScrollLeft`/`ScrollRight` aren't handled. Neither is
  Shift+wheel, which crossterm reports as `ScrollUp`/`ScrollDown` with `SHIFT`. `File.left`
  exists, but `scroll_to_cursor` recomputes it from the cursor on every key.

## Design

### Hit testing

`App` records each area as it's drawn. It already records `tree_area`, `view`, `left_view` and
`box_area`, and the picker computes its own areas from the screen. Add the help panel and the
completion list, which are drawn but not recorded today.

`App::mouse` first finds the area under the pointer. The two modal layers keep their place in
front:

- **A picker** still gets every event. Over its preview, the wheel scrolls the preview. Clicks
  outside it still do nothing.
- **The command line** swallows clicks. The wheel over the completion list scrolls it, and
  anywhere else it does nothing.

Below those, the area under the pointer decides:

| Under the pointer | Click | Wheel |
|---|---|---|
| Tree | Focuses the tree; on a file, also opens it (focus stays in the tree) | Scrolls the tree's view |
| Code (`view`, `left_view`) | Unfocuses the tree, places the cursor, drag selects, toggles a thread row | Scrolls the code |
| Comment box | Places the box's cursor | Scrolls the box |
| Help panel | Nothing | Scrolls the panel |
| Statusline, command line | Nothing | Nothing |

While the comment box is open, a click outside it is still swallowed, because the box holds an
unsaved draft. The wheel isn't swallowed: scrolling the code under an open box moves nothing you
typed.

### Focus

There are two focusable panes: the tree and the code. The comment box takes focus by opening, as
it does now. A click in the tree sets `tree.focused = true`, and a click in the code sets it to
`false`. `Tree::mouse` stops unfocusing the tree when you click a file. This is a deliberate
change from `ret`, which still opens the file and gives the keys back to it.

### Scrolling a view without moving its cursor

The tree, the box, the help panel, the completion list and the picker preview each get a `top`
that the wheel moves by 3 rows, clamped to their content.

- **Tree:** `Tree::render` stops moving `top` to the cursor on every draw. Only cursor moves
  scroll the cursor into view.
- **Comment box:** it scrolls like the code, with the cursor pulled into view as
  `scroll_lines` does.
- **Help panel:** its `top` resets whenever the menu changes.
- **Completion list:** its window moves, and the selected completion stays selected even when
  it's scrolled out of sight. Tab brings it back.
- **Picker preview:** its `top` resets whenever the selection changes.

### Horizontal scrolling

- **Which events:** `ScrollLeft`/`ScrollRight`, plus Shift+wheel, move `File.left` by 6 columns,
  and only when soft wrap is off.
- **The cursor follows:** as `scroll_lines` keeps the cursor in view vertically, a horizontal
  scroll moves the cursor to the nearest visible column on its line. Otherwise
  `scroll_to_cursor` would undo the scroll on the next draw.
- **Split view:** the old side already follows the new side's `left`, so both scroll together.
- **Comment box:** it always wraps, so it ignores sideways scrolling.
- **Terminals:** I believe iTerm2, kitty, WezTerm and Ghostty send sideways swipes as
  `ScrollLeft`/`ScrollRight`; I'm unsure about Terminal.app. Shift+wheel works in any terminal
  that reports the Shift modifier.

## Steps

One PR on a `mouse` branch, one commit per step. Each step passes `cargo fmt`, `clippy` and
`cargo test`, including the perf tests.

1. **Hit testing and focus.** Record the help panel and completion areas, add `App::area_at`, and
   route by it. Clicks focus the tree or the code, which fixes the focus bug.
2. **The tree scrolls.** Separate `top` from the cursor in `Tree`, and make the wheel scroll it.
3. **Everything else that overflows scrolls:** the comment box, the help panel, the completion
   list and the picker preview.
4. **Horizontal scrolling**, from sideways swipes and Shift+wheel.

## Tests

- A click in the tree focuses it, and the next `j` moves the tree's cursor. A click in the code
  unfocuses it, and the next `j` moves the code's cursor. This is the regression test for the bug.
- A click on a tree file opens it, and the tree keeps focus.
- The wheel over the tree scrolls it, and its cursor stays put. The wheel over the code, with the
  tree focused, scrolls the code.
- The wheel scrolls the comment box, help panel, completion list and picker preview, and each
  one's `top` resets as described above.
- A sideways swipe and Shift+wheel move `left` by 6. The cursor stays visible, and the next key
  doesn't undo the scroll. With soft wrap on, nothing happens. In split view, both sides move.
- The command line and comment box still swallow clicks outside themselves.

## Out of scope

- Dragging to resize the tree.
- A clickable scrollbar.

## As built

The work follows the plan, with four differences.

Scrolling sideways doesn't move the cursor. The plan was to pull the cursor into the visible
columns after each sideways scroll, but that failed in two cases. If the cursor's line was
shorter than the scroll, there was no visible column to move it to, so the view jumped straight
back. And a diagonal swipe on a touchpad also sends vertical wheel events, which moved the cursor
and undid the sideways scroll. Instead, the code stays scrolled sideways until a key moves the
cursor, even if the cursor is off screen in the meantime.

Scrolling right stops once the end of the widest visible line is on screen.

Clicking in the comment box moves its cursor. In insert mode, the cursor becomes a single point.
In normal mode, dragging selects text, just as it does in the code.

The four steps went in as one commit, because they all change the same routing code in
`App::mouse`.
