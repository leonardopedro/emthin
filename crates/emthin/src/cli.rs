use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "emthin",
    version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("EMTHIN_GIT_SHA"), ")")
)]
pub struct Cli {
    /// Launch an application into a figure. Repeatable: each `--spawn`
    /// is one full command line (`emthin --spawn foot --spawn
    /// "firefox --new-window https://example.com"`). Quotes and
    /// backslash escapes are honoured. Every app that maps a toplevel
    /// gets its own `\app` figure appended to the document.
    #[arg(long = "spawn", value_name = "CMD")]
    pub spawn: Vec<String>,

    /// Open this document instead of the last session's (or a new one).
    #[arg(long, value_name = "PATH")]
    pub doc: Option<std::path::PathBuf>,

    /// Nested-session state file. Without it emthin keeps its session
    /// in `$XDG_STATE_HOME/emthin/` only when it is the primary session
    /// (see `session.rs`).
    #[arg(long, value_name = "PATH")]
    pub session_file: Option<std::path::PathBuf>,

    /// Do not spawn anything; wait for an external `spawn` over IPC.
    ///
    /// Kept as the negative of the old `--standalone` flag's intent
    /// (auto-launch a child) and now the *default*: with no `--spawn`,
    /// emthin starts with an empty document.
    #[arg(long)]
    pub no_spawn: bool,

    /// Explicit IPC socket path (default: $XDG_RUNTIME_DIR/emthin-<pid>.ipc).
    #[arg(long)]
    pub ipc_path: Option<std::path::PathBuf>,

    /// Pin the Wayland display socket name (default: auto-chosen wayland-N
    /// by smithay).
    #[arg(long)]
    pub wayland_socket: Option<String>,

    /// XKB keyboard layout (e.g. "us", "de", "cn").
    #[arg(long, default_value = "")]
    pub xkb_layout: String,

    /// XKB keyboard model (e.g. "pc105").
    #[arg(long, default_value = "")]
    pub xkb_model: String,

    /// XKB layout variant (e.g. "nodeadkeys").
    #[arg(long, default_value = "")]
    pub xkb_variant: String,

    /// XKB options (e.g. "ctrl:nocaps").
    #[arg(long)]
    pub xkb_options: Option<String>,

    /// Request fullscreen for the host compositor window on startup.
    #[arg(long)]
    pub fullscreen: bool,

    /// Write tracing logs to this file instead of stderr.
    #[arg(long)]
    pub log_file: Option<std::path::PathBuf>,

    /// Pin the XWayland DISPLAY number that emthin asks
    /// xwayland-satellite to claim.
    #[arg(long)]
    pub xwayland_display: Option<u32>,

    /// Path to the `xwayland-satellite` binary. Defaults to the binary
    /// found on `$PATH`.
    #[arg(long, default_value = "xwayland-satellite")]
    pub xwayland_satellite_bin: std::path::PathBuf,

    /// Spawn a private `dbus-daemon` for embedded apps and route the
    /// broker's upstream to it instead of the host session bus.
    #[arg(long)]
    pub dbus_isolated: bool,
}

impl Cli {
    /// The `--spawn` command lines, split into `(program, args)`.
    ///
    /// An empty `--spawn ""` yields nothing rather than panicking on an
    /// empty word list.
    pub fn spawn_commands(&self) -> Vec<(String, Vec<String>)> {
        self.spawn
            .iter()
            .filter_map(|line| {
                let mut parts = split_command(line);
                // `remove(0)` panics on an empty vec.
                if parts.is_empty() {
                    None
                } else {
                    Some(parts.remove(0))
                }
                .map(|program| (program, parts))
            })
            .collect()
    }
}

/// Split one `--spawn` command line into words, honouring single and
/// double quotes and backslash escapes — enough for the app command
/// lines people actually write, without pulling in a shell (a shell
/// would be both a dependency and a security question).
///
/// An unterminated quote still quotes the rest of the line: a
/// half-typed `"` should still get the app launched rather than
/// refusing to start.
pub fn split_command(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has_token = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(q) if c == '\\' && q == '"' => {
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            }
            Some(_) => cur.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                has_token = true;
            }
            None if c == '\\' => {
                if let Some(next) = chars.next() {
                    cur.push(next);
                    has_token = true;
                }
            }
            None if c.is_whitespace() => {
                if has_token || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            None => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token || !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_plain_words() {
        assert_eq!(split_command("foot"), ["foot"]);
        assert_eq!(
            split_command("firefox --new-window https://example.com"),
            ["firefox", "--new-window", "https://example.com"]
        );
    }

    #[test]
    fn collapses_runs_of_whitespace() {
        assert_eq!(
            split_command("  foot \t -T \n xterm  "),
            ["foot", "-T", "xterm"]
        );
    }

    #[test]
    fn honours_quotes() {
        assert_eq!(
            split_command("foot -T \"xterm-256color\""),
            ["foot", "-T", "xterm-256color"]
        );
        assert_eq!(split_command("sh -c 'echo hi'"), ["sh", "-c", "echo hi"]);
        // A quoted empty string is still an argument.
        assert_eq!(split_command("foot ''"), ["foot", ""]);
    }

    #[test]
    fn backslash_escapes_spaces() {
        assert_eq!(split_command(r"foot my\ file.txt"), ["foot", "my file.txt"]);
    }

    #[test]
    fn unterminated_quote_keeps_the_rest() {
        // A half-typed quote still quotes: better a launched app with a
        // slightly wrong argv than no app at all.
        assert_eq!(split_command("foot \"bar"), ["foot", "bar"]);
    }

    #[test]
    fn empty_line_yields_nothing() {
        assert!(split_command("").is_empty());
        assert!(split_command("   ").is_empty());
    }
}
