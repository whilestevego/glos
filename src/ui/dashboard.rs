//! The dashboard (`glos dashboard`, `' l`): every open review on this machine, with what its
//! agent is doing and what's waiting on you. `ret` opens one. It's a picker, so typing filters.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::{DefaultTerminal, layout::Rect};
use rusqlite::Connection;

use super::{
    component::{Component, Cx, Msg, Outcome},
    keymap::Key,
    picker::{self, Item, Picker, Target},
};
use crate::{
    review::{self, Review},
    source::Source,
    theme::Theme,
};

pub const COLUMNS: [&str; 7] = [
    "state", "agent", "unsent", "unread", "open", "outdated", "review",
];

/// What the review's agent is doing, from the store.
fn state(r: &Review, rounds: &[review::Round], unread: usize) -> &'static str {
    if r.requested_at.is_some() {
        "waiting for review"
    } else if unread > 0 {
        "replied"
    } else {
        match rounds.last() {
            Some(round) if round.delivered_to.is_none() => "submitted, not picked up",
            Some(_) => "working",
            None => "-",
        }
    }
}

/// One row per open review, most recently changed first.
pub fn items(conn: &Connection, theme: &Theme) -> Result<Vec<Item>> {
    let sessions: Vec<(i64, String, String)> = conn
        .prepare("SELECT pid, agent_name, folder FROM sessions ORDER BY attached_at DESC")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let plain = theme.get("ui.text");
    let mut items = vec![];
    for r in Review::list(conn, false)? {
        let threads = review::threads(conn, &r.id)?;
        let rounds = review::rounds(conn, &r.id)?;
        let unsent = review::unsent(conn, &r.id)?;
        let unread = threads.iter().filter(|t| t.unread()).count();
        let count = |s: &str| threads.iter().filter(|t| t.status == s).count();
        let mut agents: Vec<&str> = sessions
            .iter()
            .filter(|(pid, _, folder)| {
                Path::new(folder).starts_with(&r.folder) && review::alive(*pid)
            })
            .map(|(_, name, _)| name.as_str())
            .collect();
        agents.dedup();
        let agent = if agents.is_empty() {
            "-".into()
        } else {
            agents.join(", ")
        };
        let n = |n: usize, key: &str| {
            let style = if n > 0 { theme.get(key) } else { plain };
            (n.to_string(), style)
        };
        let st = state(&r, &rounds, unread);
        let st_style = if st == "waiting for review" {
            theme.get("warning")
        } else {
            plain
        };
        items.push(Item {
            tags: vec![
                (st.into(), st_style),
                (agent, plain),
                n(unsent, "warning"),
                n(unread, "info"),
                n(count("open"), "info"),
                n(count("outdated"), "hint"),
            ],
            text: home(&r.describe()),
            target: Target::Review(r.id.clone()),
        });
    }
    Ok(items)
}

/// Paths under your home folder as `~/…`, to keep rows short.
fn home(s: &str) -> String {
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() => s.replace(&format!("{}/", h.trim_end_matches('/')), "~/"),
        _ => s.to_string(),
    }
}

pub fn picker(conn: &Connection, theme: &Theme) -> Result<Picker> {
    let mut p = Picker::new(COLUMNS.to_vec(), items(conn, theme)?);
    p.preview = false;
    Ok(p)
}

pub fn is_dashboard(p: &Picker) -> bool {
    p.columns == COLUMNS
}

/// The changes of the review with this id, to open it.
pub fn source_of(conn: &Connection, id: &str) -> Result<Source> {
    Review::get(conn, id)?
        .with_context(|| format!("no review {id}"))?
        .source()
}

/// `glos dashboard` on its own, before any review is open: the review chosen, or `None` to quit.
pub fn run(
    terminal: &mut DefaultTerminal,
    conn: &Connection,
    theme: &Theme,
) -> Result<Option<Source>> {
    let mut list = picker(conn, theme)?;
    let mut keys: Option<Picker> = None;
    let mut status: Option<String> = None;
    let version = || -> i64 {
        conn.pragma_query_value(None, "data_version", |r| r.get(0))
            .unwrap_or(0)
    };
    let mut seen = version();
    let keymaps = super::keymap::Keymaps::default();
    let mut screen = Rect::default();
    loop {
        terminal.draw(|f| {
            screen = f.area();
            let buf = f.buffer_mut();
            buf.set_style(screen, theme.get("ui.background"));
            let cx = Cx {
                screen,
                theme,
                state: None,
            };
            keys.as_mut().unwrap_or(&mut list).render(buf, screen, &cx);
            let line = status.clone().unwrap_or_else(|| {
                "Reviews on this machine · ret opens · esc quits · C-g keys".into()
            });
            let style = theme.get(if status.is_some() { "error" } else { "ui.text" });
            let y = screen.bottom().saturating_sub(1);
            buf.set_stringn(
                0,
                y,
                super::view::clean(&line),
                screen.width as usize,
                style,
            );
        })?;
        if event::poll(Duration::from_millis(500))? {
            let cx = Cx {
                screen,
                theme,
                state: None,
            };
            let outcome = match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    let key = Key::from(k);
                    status = None;
                    let picker_keys = keymaps.get(super::keymap::Context::Picker);
                    if matches!(picker_keys.get(key), Some(super::keymap::Node::Cmd(c)) if c.name == "command_palette")
                    {
                        keys = match keys {
                            Some(_) => None,
                            None => Some(picker::keys_palette(theme, &keymaps)),
                        };
                        continue;
                    }
                    keys.as_mut().unwrap_or(&mut list).press(&keymaps, key, &cx)
                }
                Event::Mouse(m) => keys.as_mut().unwrap_or(&mut list).mouse(m, screen, &cx),
                _ => Outcome::Consumed,
            };
            match outcome {
                Outcome::Ignored | Outcome::Consumed => {}
                Outcome::Msg(Msg::Close) if keys.is_some() => keys = None,
                Outcome::Msg(Msg::Close) => return Ok(None),
                Outcome::Msg(Msg::Open(Target::Review(id))) => match source_of(conn, &id) {
                    Ok(s) => return Ok(Some(s)),
                    Err(e) => status = Some(format!("{e:#}")),
                },
                Outcome::Msg(Msg::Open(_)) => keys = None,
                Outcome::Msg(_) => {} // a picker sends nothing else
            }
        }
        let v = version();
        if v != seen {
            seen = v;
            list.set_items(items(conn, theme)?);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{review::Verdict, store};

    #[test]
    fn rows_say_what_the_agent_does_and_what_waits_on_you() {
        let dir = tempfile::tempdir().unwrap();
        let conn = store::open(&dir.path().join("glos.db")).unwrap();
        let theme = Theme::load("default", crate::theme::ColorSupport::TrueColor).unwrap();
        let mk = |name: &str| {
            let (o, n) = (
                dir.path().join(format!("{name}.old")),
                dir.path().join(name),
            );
            std::fs::write(&o, "a\n").unwrap();
            std::fs::write(&n, "a\nb\nc\n").unwrap();
            Review::open(&conn, &Source::diff(&o, &n).unwrap()).unwrap()
        };
        let a = mk("a.rs");
        let b = mk("b.rs");
        let row = |id: &str| -> Vec<String> {
            let items = items(&conn, &theme).unwrap();
            let it = items
                .iter()
                .find(|i| i.target == Target::Review(id.into()))
                .unwrap();
            let mut v: Vec<String> = it.tags.iter().map(|(t, _)| t.clone()).collect();
            v.push(it.text.clone());
            v
        };
        assert_eq!(row(&a.id)[..6], ["-", "-", "0", "0", "0", "0"]);
        assert!(
            row(&a.id)[6].starts_with("a.rs.old → a.rs in "),
            "{:?}",
            row(&a.id)
        );

        a.request(&conn, "claude", "done", &[]).unwrap();
        assert_eq!(row(&a.id)[0], "waiting for review");
        let anchor = review::Anchor {
            path: "a.rs".into(),
            start: 2,
            end: 2,
            side: review::Side::New,
        };
        let t = review::comment(
            &conn,
            &a.id,
            Some((&anchor, "a\nb\nc\n")),
            review::YOU,
            "why?",
        )
        .unwrap();
        assert_eq!(row(&a.id)[2..5], ["1", "0", "1"]);
        review::submit(&conn, &a.id, Verdict::Changes, &[], None).unwrap();
        assert_eq!(row(&a.id)[0], "submitted, not picked up");
        review::take_delivery(&conn, &a.id, "claude")
            .unwrap()
            .unwrap();
        assert_eq!(row(&a.id)[0], "working");
        review::reply(&conn, &t, "claude", "because").unwrap();
        assert_eq!(row(&a.id)[0], "replied");
        assert_eq!(row(&a.id)[3], "1", "unread");

        // An attached agent serving the folder, by name; a gone one isn't listed.
        conn.execute(
            "INSERT INTO sessions VALUES (?1, 'claude', ?2, 'x', 'x'), (999999999, 'ghost', ?2, 'x', 'x')",
            rusqlite::params![std::process::id(), dir.path().canonicalize().unwrap().to_string_lossy()],
        )
        .unwrap();
        assert_eq!(row(&b.id)[1], "claude");

        // Closed reviews leave the dashboard.
        review::close(&conn, &b.id).unwrap();
        assert!(items(&conn, &theme).unwrap().len() == 1);
        assert!(is_dashboard(&picker(&conn, &theme).unwrap()));
        assert!(source_of(&conn, "nope").is_err());
        assert!(source_of(&conn, &a.id).is_ok());
    }
}
