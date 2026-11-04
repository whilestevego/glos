//! The system clipboard, as Helix reaches it: `pbcopy`, `wl-copy`, `xclip` or `xsel`, or the
//! terminal itself (OSC 52) over SSH or when there's no other way.

use std::{
    io::Write,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};

/// Which way the clipboard is reached here.
pub fn provider() -> &'static str {
    let has = |cmd: &str| which(cmd);
    if std::env::var_os("SSH_TTY").is_some() || std::env::var_os("SSH_CONNECTION").is_some() {
        "termcode"
    } else if cfg!(target_os = "macos") && has("pbcopy") {
        "pbcopy"
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some() && has("wl-copy") {
        "wl-copy"
    } else if std::env::var_os("DISPLAY").is_some() && has("xclip") {
        "xclip"
    } else if std::env::var_os("DISPLAY").is_some() && has("xsel") {
        "xsel"
    } else {
        "termcode"
    }
}

pub fn copy(text: &str) -> Result<()> {
    let (cmd, args): (&str, &[&str]) = match provider() {
        "pbcopy" => ("pbcopy", &[]),
        "wl-copy" => ("wl-copy", &["--type", "text/plain"]),
        "xclip" => ("xclip", &["-i", "-selection", "clipboard"]),
        "xsel" => ("xsel", &["-i", "-b"]),
        _ => {
            // OSC 52: the terminal sets its own clipboard.
            let mut out = std::io::stdout();
            write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()))?;
            return Ok(out.flush()?);
        }
    };
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("running {cmd}"))?;
    child
        .stdin
        .take()
        .context("no stdin")?
        .write_all(text.as_bytes())?;
    if !child.wait()?.success() {
        bail!("{cmd} failed");
    }
    Ok(())
}

fn which(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
