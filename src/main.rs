mod cli;
mod clipboard;
mod config;
mod diff;
mod diff_words;
mod doctor;
mod launch;
mod mcp;
mod reanchor;
mod review;
mod source;
mod store;
mod theme;
mod ui;

use std::{path::PathBuf, process::ExitCode};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use etcetera::BaseStrategy;

use source::Source;

/// A terminal code reviewer for work done by coding agents.
///
/// Plain `glos` reviews the current git repo: the branch since it left its base, uncommitted and
/// untracked files included.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// The database to use instead of `$XDG_DATA_HOME/glos/glos.db`.
    #[arg(long, env = "GLOS_DB", global = true)]
    db: Option<PathBuf>,

    /// More detail in the log (`~/.local/state/glos/glos.log`); repeat for more.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// JSON output for scripts and agents (`list`, `show`, `submit`).
    #[arg(long, global = true)]
    json: bool,

    /// Open the review with this id (see `glos list`), wherever it is.
    #[arg(long)]
    review: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Review two files or two folders, like `git diff --no-index`; or git revisions, like
    /// `git diff`: one revision against your files, or two against each other (read-only).
    /// A path that exists wins over a revision of the same name.
    Diff { old: String, new: Option<String> },
    /// Check glos's setup, like `hx --health`.
    Doctor,
    /// Run the MCP server for an agent, over stdio (`claude mcp add glos -- glos mcp`).
    Mcp,
    /// Reviews on this machine: open ones, or every one with `--all`.
    List {
        #[arg(long)]
        all: bool,
    },
    /// Name what to review, as an agent without MCP would: the review of exactly that, made if
    /// there's none yet. Prints its id, which every other command takes as `--review`.
    OpenReview {
        #[command(subcommand)]
        target: TargetArg,
    },
    /// A review's rounds, threads and comments.
    Show {
        #[command(flatten)]
        review: ReviewArg,
    },
    /// Reply on a thread, as an agent without MCP would.
    Reply {
        #[command(flatten)]
        review: ReviewArg,
        thread: String,
        body: String,
        #[command(flatten)]
        author: Author,
    },
    /// Resolve a thread, with a note saying what was done.
    Resolve {
        #[command(flatten)]
        review: ReviewArg,
        thread: String,
        note: String,
        #[command(flatten)]
        author: Author,
    },
    /// Reopen a resolved or outdated thread.
    Reopen {
        #[command(flatten)]
        review: ReviewArg,
        thread: String,
        note: Option<String>,
        #[command(flatten)]
        author: Author,
    },
    /// Leave a note on lines of a file (`12` or `12-14`) before review.
    Annotate {
        #[command(flatten)]
        review: ReviewArg,
        path: String,
        lines: String,
        body: String,
        #[command(flatten)]
        author: Author,
    },
    /// Close a review you're done with.
    Close {
        #[command(flatten)]
        review: ReviewArg,
    },
    /// Every open review on this machine; `ret` opens one.
    Dashboard,
    /// Open glos on a review the way an agent's request does: with `open-command` from
    /// config.toml, unless glos is already open on it.
    Open {
        #[command(flatten)]
        review: ReviewArg,
    },
    /// Delete archived reviews older than `--days`, and file content nothing uses any more.
    Gc {
        #[arg(long, default_value_t = 30)]
        days: u32,
    },
    /// Shell completions: `glos completions zsh > ~/.zfunc/_glos`.
    Completions { shell: clap_complete::Shell },
    /// The man page, in roff: `glos man > glos.1`.
    #[command(hide = true)]
    Man,
    /// Submit the unsent comments as a round: approve, changes or comment.
    Submit {
        #[arg(value_parser = parse_verdict)]
        verdict: review::Verdict,
        #[command(flatten)]
        review: ReviewArg,
    },
}

/// Which review a command acts on: always named, never guessed from the folder.
#[derive(clap::Args)]
struct ReviewArg {
    /// The review's id, or the start of it (`glos list` shows them; `glos open-review` makes
    /// one).
    #[arg(long = "review", value_name = "ID")]
    id: String,
}

/// What `glos open-review` reviews.
#[derive(Subcommand)]
enum TargetArg {
    /// A git branch against its base, in the worktree that has it checked out.
    Git {
        folder: std::path::PathBuf,
        branch: String,
        base: String,
    },
    /// Two files, or two folders.
    Diff {
        old: std::path::PathBuf,
        new: std::path::PathBuf,
    },
}

#[derive(clap::Args)]
struct Author {
    /// Who's writing: the agent's name.
    #[arg(long = "as", default_value = "agent")]
    name: String,
}

fn parse_verdict(s: &str) -> Result<review::Verdict, String> {
    review::Verdict::parse(s).ok_or_else(|| "approve, changes or comment".into())
}

/// XDG folders on macOS too, as Helix does.
fn default_db() -> Result<PathBuf> {
    Ok(etcetera::base_strategy::Xdg::new()?
        .data_dir()
        .join("glos/glos.db"))
}

/// When glos started, for the first-screen time in the log (target: under 100 ms).
pub static STARTED: std::sync::LazyLock<std::time::Instant> =
    std::sync::LazyLock::new(std::time::Instant::now);

fn main() -> ExitCode {
    std::sync::LazyLock::force(&STARTED);
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("glos: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Logs never go to the terminal: in the UI they'd corrupt the screen, and in `glos mcp`
/// stdout carries the protocol. They go to `$XDG_STATE_HOME/glos/glos.log`.
fn init_log(verbose: u8) -> Result<PathBuf> {
    let path = etcetera::base_strategy::Xdg::new()?
        .state_dir()
        .unwrap_or_default()
        .join("glos/glos.log");
    std::fs::create_dir_all(path.parent().unwrap())?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let level = match verbose {
        0 => tracing::Level::WARN,
        1 => tracing::Level::INFO,
        2 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(file))
        .with_ansi(false)
        .with_max_level(level)
        .init();
    Ok(path)
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let log = init_log(cli.verbose).ok();
    let db = match cli.db {
        Some(p) => p,
        None => default_db()?,
    };
    match &cli.command {
        Some(Command::Doctor) => {
            doctor::run(&mut std::io::stdout(), &db, log.as_deref())?;
            return Ok(ExitCode::SUCCESS);
        }
        Some(Command::Completions { shell }) => {
            use clap::CommandFactory;
            clap_complete::generate(*shell, &mut Cli::command(), "glos", &mut std::io::stdout());
            return Ok(ExitCode::SUCCESS);
        }
        Some(Command::Man) => {
            use clap::CommandFactory;
            clap_mangen::Man::new(Cli::command()).render(&mut std::io::stdout())?;
            return Ok(ExitCode::SUCCESS);
        }
        _ => {}
    }
    let conn = store::open(&db)?;
    let cwd = std::env::current_dir()?.canonicalize()?;
    let out = &mut std::io::stdout();
    match &cli.command {
        Some(Command::Mcp) => {
            mcp::run(conn, &db, config::load().0.open_command)?;
            return Ok(ExitCode::SUCCESS);
        }
        Some(Command::Close { review }) => {
            cli::close(&conn, &review.id, cli.json, out)?;
            return Ok(ExitCode::SUCCESS);
        }
        Some(Command::Gc { days }) => {
            cli::gc(&conn, *days, cli.json, out)?;
            return Ok(ExitCode::SUCCESS);
        }
        Some(Command::Open { review }) => {
            let command = config::load().0.open_command;
            let opened = cli::open(&conn, &review.id, command.as_deref(), &db, out)?;
            return Ok(if opened {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            });
        }
        Some(Command::List { all }) => {
            cli::list(&conn, *all, cli.json, out)?;
            return Ok(ExitCode::SUCCESS);
        }
        Some(Command::Show { review }) => {
            cli::show(&conn, &review.id, cli.json, out)?;
            return Ok(ExitCode::SUCCESS);
        }
        Some(Command::OpenReview { target }) => {
            // Paths given on the command line are the shell's: relative to where it runs.
            let abs = |p: &std::path::Path| -> Result<String> {
                Ok(std::path::absolute(p)?.to_string_lossy().into_owned())
            };
            let target = match target {
                TargetArg::Git {
                    folder,
                    branch,
                    base,
                } => mcp::Target::Git {
                    folder: abs(folder)?,
                    branch: branch.clone(),
                    base: base.clone(),
                },
                TargetArg::Diff { old, new } => mcp::Target::Diff {
                    old: abs(old)?,
                    new: abs(new)?,
                },
            };
            cli::open_review(conn, &cwd, target, cli.json, out)?;
            return Ok(ExitCode::SUCCESS);
        }
        Some(
            Command::Reply { author, review, .. }
            | Command::Resolve { author, review, .. }
            | Command::Reopen { author, review, .. }
            | Command::Annotate { author, review, .. },
        ) => {
            use cli::AgentAction as A;
            let (name, id) = (author.name.clone(), review.id.clone());
            let action = match cli.command.unwrap() {
                Command::Reply { thread, body, .. } => A::Reply { thread, body },
                Command::Resolve { thread, note, .. } => A::Resolve { thread, note },
                Command::Reopen { thread, note, .. } => A::Reopen {
                    thread,
                    note: note.unwrap_or_default(),
                },
                Command::Annotate {
                    path, lines, body, ..
                } => A::Annotate { path, lines, body },
                _ => unreachable!(),
            };
            cli::agent(conn, &cwd, &id, &name, action, cli.json, out)?;
            return Ok(ExitCode::SUCCESS);
        }
        Some(Command::Submit { verdict, review }) => {
            cli::submit(&conn, &review.id, *verdict, cli.json, out)?;
            return Ok(ExitCode::SUCCESS);
        }
        _ => {}
    }

    let (config, config_error) = config::load();
    let start = match (&cli.command, &cli.review) {
        (None, Some(id)) => {
            let r = review::Review::get(&conn, id)?
                .with_context(|| format!("no review {id}; see glos list"))?;
            Some(ui::Start::Review(Box::new(r.source()?)))
        }
        (Some(Command::Dashboard), _) => {
            review::archive_finished(&conn)?;
            if review::Review::list(&conn, false)?.is_empty() {
                println!("No open reviews. Run glos in a git repo, or glos diff <old> <new>.");
                return Ok(ExitCode::SUCCESS);
            }
            Some(ui::Start::Dashboard)
        }
        _ => None,
    };
    let source = match start {
        Some(ui::Start::Review(s)) => *s,
        Some(ui::Start::Dashboard) => {
            show_config_error(config_error);
            ui::run(ui::Start::Dashboard, &config, conn)?;
            return Ok(ExitCode::SUCCESS);
        }
        None => match cli.command {
            Some(Command::Diff { old, new }) => {
                let is_path = |p: &str| std::path::Path::new(p).exists();
                match new {
                    Some(new) if is_path(&old) && is_path(&new) => {
                        Source::diff(old.as_ref(), new.as_ref())?
                    }
                    None if is_path(&old) => {
                        eprintln!(
                            "glos: give two files or folders, or a git revision (glos diff HEAD~1)"
                        );
                        return Ok(ExitCode::from(2));
                    }
                    new => Source::revs(&cwd, &old, new.as_deref())?,
                }
            }
            Some(_) => unreachable!(),
            None => match Source::git(&std::env::current_dir()?, None) {
                Ok(s) => s,
                Err(e) if gix::discover(".").is_err() => {
                    eprintln!("glos: not in a git repository ({e:#}).");
                    eprintln!("To review two files or folders: glos diff <old> <new>");
                    return Ok(ExitCode::from(2));
                }
                Err(e) => return Err(e),
            },
        },
    };
    if source.changes.is_empty() {
        match &source.git {
            Some(_) if source.revs.is_some() => {
                let (o, n) = source.revs.as_ref().unwrap();
                let n = n.as_deref().unwrap_or("your files");
                println!("Nothing to review: no changes between {o} and {n}.");
            }
            Some(g) => println!("Nothing to review: no changes since {}.", g.base),
            None => println!("Nothing to review: no differences."),
        }
        return Ok(ExitCode::SUCCESS);
    }
    show_config_error(config_error);
    ui::run(ui::Start::Review(Box::new(source)), &config, conn)?;
    Ok(ExitCode::SUCCESS)
}

/// As Helix: say what's wrong with the config, then carry on with the defaults.
fn show_config_error(error: Option<String>) {
    if let Some(e) = error {
        eprintln!("Bad config: {e}");
        eprintln!("Press <ENTER> to continue with the default config");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}
