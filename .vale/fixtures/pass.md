# Pass

Run `cargo xtask prose`. The check reads each line that a branch adds.

A code span is skipped: `a — b`.

| Column | Text |
|---|---|
| One | This table cell has many words in it, more than twenty-five of them, and Vale skips it because a table is in the skipped scopes list. |

```text
A code block is skipped too — even a long one with crucial words.
```

A leap year has 366 days only when the rule allows it.
