//! Hand-rolled argument parsing (the workspace carries no clap): a
//! subcommand plus a handful of flags, fully unit-tested.

use std::path::PathBuf;

use crate::report::ReportKind;
use crate::session::ConnectTarget;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// `guiwright repl --uds <path> | --tcp <addr>` — the §13 live session
    /// debugger. `--json` switches the output mode to one machine-readable
    /// JSON document per step (the m11/m13 records' named CLI milestone —
    /// the lines carry their kind word, the exit flag rides the document).
    Repl { target: ConnectTarget, json: bool },
    /// `guiwright show-trace <archive.zip> [--out <path>] [--open]` — the
    /// §12 deep-link target.
    ShowTrace {
        archive: PathBuf,
        out: Option<PathBuf>,
        open: bool,
    },
    /// `guiwright live-html <target> [--out <path>] [--duration-ms N]
    /// [--test NAME]` — the §12 live-mode emitter (m87): attach the live
    /// feed to a running session over any door and emit the scrubber
    /// HTML mid-session, stamped `gwtrace/v1 (live)`.
    LiveHtml {
        target: ConnectTarget,
        out: Option<PathBuf>,
        duration_ms: Option<u64>,
        test: Option<String>,
    },
    /// `guiwright bless <dir> [--dry-run]` — the §10 bless flow as a CLI.
    Bless { dir: PathBuf, dry_run: bool },
    /// `guiwright install-deps [--run]` — the §11 CI provisioning printer.
    InstallDeps { run: bool },
    /// `guiwright codegen [--uds <path> | --tcp <addr>] [--attach-ms N]
    /// [--duration-ms N] [--out <path>] <app-cmd…>` — the §13 standalone
    /// codegen session (the m4 §4 deferred row): spawn the app with the
    /// §5.3 `GUIWRIGHT_DRIVER` launch-mode env, wait for its door, run
    /// the overlay + recorder session, and hand the kit code back when
    /// the window ends (deadline, Ctrl-C, or the app closing).
    Codegen {
        /// Door override; `None` picks a fresh temp UDS path.
        door: Option<ConnectTarget>,
        /// Door-wait deadline in ms (`--attach-ms`).
        attach_ms: Option<u64>,
        /// Capture window in ms (`--duration-ms`); `None` = until the
        /// door closes (the Ctrl-C finish on unix).
        duration_ms: Option<u64>,
        /// Also write the generated kit code here (`--out`).
        out: Option<PathBuf>,
        /// The app command: `command[0]` is the program, the rest its
        /// argv (everything after the first non-flag token).
        command: Vec<String>,
    },
    /// `guiwright test [PATHS…]` — the §16 v0.5 own runner: discovers
    /// `gwtest/v1` scenario specs and drives them over GWP with workers,
    /// retries, sharding and reports.
    Test {
        /// Spec files or directories scanned recursively for
        /// `*.spec.toml` (empty = the `tests/guiwright` convention).
        paths: Vec<PathBuf>,
        /// CLI target override (beats every spec file's `target`).
        target: Option<ConnectTarget>,
        /// Parallel scenario workers (default 1).
        workers: usize,
        /// Global retry override for failed scenarios.
        retries: Option<u32>,
        /// Global step deadline override (ms).
        timeout_ms: Option<u64>,
        /// `--shard k/n` — interleaved deterministic split (§11).
        shard: Option<(u32, u32)>,
        /// Print the discovered scenarios and exit (shard-aware).
        list: bool,
        /// Per-scenario progress lines suppressed.
        quiet: bool,
        /// File reports to write (repeatable `--report <kind>`).
        reports: Vec<ReportKind>,
        /// Report output directory (default `guiwright-report/`).
        out: Option<PathBuf>,
        /// Record a per-scenario trace (the M18 live feed over a second
        /// connection, finalized as the M23 scrubber HTML) and deep-link
        /// it from the reports (the m5 §4 deferral, closed).
        trace: bool,
    },
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CliError {
    #[error("usage: {0}")]
    Usage(&'static str),
    #[error("unknown command `{0}` — try `guiwright --help`")]
    UnknownCommand(String),
    #[error("{0}")]
    BadValue(String),
}

/// The `--help` text (also the REPL-external command reference).
pub fn help_text() -> &'static str {
    "guiwright — Playwright-grade debugging for Rust GUI apps (§5.2)

USAGE:
    guiwright <COMMAND>

COMMANDS:
    repl <target>         Attach to a live remote-mode session (§13).
                          <target>: uds:///path/to.sock | tcp://host:port
                                  | ws://host:port (bare paths and
                                  host:port also work)
                          --json: one machine-readable JSON document per
                                  step (the lines carry their kind word;
                                  the m11/m13 records' output mode)
    show-trace <archive>  Render a gwtrace/v1 zip into trace.html (§12).
        [--out <path>]    Output path (default: next to the archive).
        [--open]          Open the rendered file with the platform opener.
    live-html <target>    Attach to a RUNNING session over any door and
        [--out <path>]    emit the live scrubber HTML mid-session (§12
        [--duration-ms N] live mode, m87; stamped gwtrace/v1 (live);
        [--test NAME]     default out: guiwright-live.html, window:
                          1000ms, ending early once the session goes
                          quiet).
    bless <dir>           Promote .new.png artifacts onto their baselines
        [--dry-run]       (§10 GUIWRIGHT_BLESS flow, for CI artifacts).
    install-deps          Print the §11 CI provisioning commands for the
        [--run]           host OS (--run executes them).
    codegen <app-cmd…>    Launch the app and record a §13 codegen
        [--uds <path>]    session: the §5.3 GUIWRIGHT_DRIVER door env is
        [--tcp <addr>]    injected, the launcher waits for the door
        [--attach-ms N]   (--attach-ms, default 10000), enables the M8
        [--duration-ms N] overlay + recorder, and stops when the window
        [--out <path>]    ends — --duration-ms, Ctrl-C, or the app
                          closing (the last one loses the server-side
                          recording). Launcher flags precede the app
                          command; everything after the app program
                          belongs to the app (`--` ends them early).
    test [PATHS…]         Run gwtest/v1 scenario specs against a live app
                          (the §16 v0.5 own runner; default PATH:
                          tests/guiwright, scanned for *.spec.toml).
        [--uds <path> | --tcp <addr> | --ws <addr>]   Target override (beats spec files).
        [--workers N]     Parallel scenario workers (default 1).
        [--retries N]     Re-run failed scenarios (default: spec value).
        [--timeout <ms>]  Step deadline override for the whole run.
        [--shard k/n]     Deterministic interleaved split (§11, one app
                          per shard).
        [--list]          Print the discovered scenarios and exit.
        [--report <kind>] list | json | junit | html (repeatable; files
                          written under --out).
        [--out <dir>]     Report directory (default guiwright-report/).
        [--trace]         Record a per-scenario trace beside the reports
                          and deep-link it (the m5 §4 deferral, closed).
        [-q | --quiet]    Suppress per-scenario progress lines.
    --help | --version

Run `guiwright repl` and type `help` for the in-session command list."
}

/// Parse `std::env::args().skip(1)`-style arguments: one dispatch arm per
/// subcommand, each delegated to its `parse_*_args` helper.
pub fn parse_args(argv: &[String]) -> Result<Command, CliError> {
    let Some(first) = argv.first() else {
        return Err(CliError::Usage(
            "a command is required — try `guiwright --help`",
        ));
    };
    match first.as_str() {
        "--help" | "-h" | "help" => Err(CliError::Usage(help_text())),
        "--version" | "-V" => Err(CliError::Usage(concat!(
            "guiwright ",
            env!("CARGO_PKG_VERSION")
        ))),
        "repl" => parse_repl_args(&argv[1..]),
        "show-trace" => parse_show_trace_args(&argv[1..]),
        "live-html" => parse_live_html_args(&argv[1..]),
        "bless" => parse_bless_args(&argv[1..]),
        "install-deps" => parse_install_deps_args(&argv[1..]),
        "codegen" => parse_codegen_args(&argv[1..]),
        "test" => parse_test_args(&argv[1..]),
        other => Err(CliError::UnknownCommand(other.to_string())),
    }
}

/// `guiwright live-html <target> [--out <path>] [--duration-ms N]
/// [--test NAME]` — the target takes the same forms the REPL's does
/// (`uds://`, `tcp://`, `ws://`, bare path/addr).
fn parse_live_html_args(rest: &[String]) -> Result<Command, CliError> {
    let mut target = None;
    let mut rest: &[String] = rest;
    while target.is_none() {
        let Some(flag) = rest.first() else {
            break;
        };
        match flag.as_str() {
            "--uds" => {
                let value = rest
                    .get(1)
                    .ok_or(CliError::Usage("--uds needs a socket path"))?;
                target = Some(ConnectTarget::Uds(PathBuf::from(value)));
                rest = &rest[2..];
            }
            "--tcp" => {
                let value = rest
                    .get(1)
                    .ok_or(CliError::Usage("--tcp needs a host:port address"))?;
                target = Some(ConnectTarget::Tcp(value.clone()));
                rest = &rest[2..];
            }
            "--ws" => {
                let value = rest
                    .get(1)
                    .ok_or(CliError::Usage("--ws needs a ws://host:port address"))?;
                target = Some(ConnectTarget::Ws(value.clone()));
                rest = &rest[2..];
            }
            other => {
                // Bare target: uds:///…, tcp://…, ws://…, /path or host:port.
                // The typed parse error names the scheme that was wrong
                // (`TargetParseError`, session.rs) — carried verbatim,
                // the REPL parser's shape, never flattened to a generic
                // "cannot parse" line.
                target = Some(
                    ConnectTarget::parse(other).map_err(|e| CliError::BadValue(e.to_string()))?,
                );
                rest = &rest[1..];
            }
        }
    }
    let target = target.ok_or(CliError::Usage("live-html needs a session target"))?;
    let mut out = None;
    let mut duration_ms = None;
    let mut test = None;
    while let Some(flag) = rest.first() {
        match flag.as_str() {
            "--out" => {
                let value = rest.get(1).ok_or(CliError::Usage("--out needs a path"))?;
                out = Some(PathBuf::from(value));
                rest = &rest[2..];
            }
            "--duration-ms" => {
                let value = rest
                    .get(1)
                    .ok_or(CliError::Usage("--duration-ms needs a number"))?;
                duration_ms = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| CliError::BadValue(format!("--duration-ms {value}")))?,
                );
                rest = &rest[2..];
            }
            "--test" => {
                let value = rest.get(1).ok_or(CliError::Usage("--test needs a name"))?;
                test = Some(value.clone());
                rest = &rest[2..];
            }
            other => return Err(CliError::UnknownCommand(other.to_string())),
        }
    }
    Ok(Command::LiveHtml {
        target,
        out,
        duration_ms,
        test,
    })
}

fn parse_repl_args(rest: &[String]) -> Result<Command, CliError> {
    {
        let mut target = None;
        let mut json = false;
        let mut rest: &[String] = rest;

        while let Some(flag) = rest.first() {
            match flag.as_str() {
                "--uds" => {
                    let value = rest
                        .get(1)
                        .ok_or(CliError::Usage("--uds needs a socket path"))?;
                    target = Some(ConnectTarget::Uds(PathBuf::from(value)));
                    rest = &rest[2..];
                }
                "--tcp" => {
                    let value = rest
                        .get(1)
                        .ok_or(CliError::Usage("--tcp needs a host:port address"))?;
                    target = Some(ConnectTarget::Tcp(value.clone()));
                    rest = &rest[2..];
                }
                "--ws" => {
                    let value = rest
                        .get(1)
                        .ok_or(CliError::Usage("--ws needs a ws://host:port address"))?;
                    target = Some(ConnectTarget::Ws(value.clone()));
                    rest = &rest[2..];
                }
                "--json" => {
                    // The machine-readable output mode (the m11/m13 records'
                    // named CLI milestone): one JSON document per step, the
                    // kind word carrying what the text mode prefixes.
                    json = true;
                    rest = &rest[1..];
                }
                other => {
                    // Bare target: uds:///…, tcp://…, ws://…, /path or host:port.
                    target = Some(
                        ConnectTarget::parse(other)
                            .map_err(|e| CliError::BadValue(e.to_string()))?,
                    );
                    rest = &rest[1..];
                }
            }
        }
        let target = target.ok_or(CliError::Usage(
            "repl needs a target: --uds <path> | --tcp <addr> | --ws <addr> | <target>",
        ))?;
        Ok(Command::Repl { target, json })
    }
}

fn parse_show_trace_args(rest: &[String]) -> Result<Command, CliError> {
    {
        let mut archive = None;
        let mut rest: &[String] = rest;
        let mut out = None;
        let mut open = false;

        while let Some(flag) = rest.first() {
            match flag.as_str() {
                "--out" => {
                    let value = rest.get(1).ok_or(CliError::Usage("--out needs a path"))?;
                    out = Some(PathBuf::from(value));
                    rest = &rest[2..];
                }
                "--open" => {
                    open = true;
                    rest = &rest[1..];
                }
                other if archive.is_none() && !other.starts_with("--") => {
                    archive = Some(PathBuf::from(other));
                    rest = &rest[1..];
                }
                other => return Err(CliError::UnknownCommand(other.to_string())),
            }
        }
        let archive = archive.ok_or(CliError::Usage("show-trace needs an archive path"))?;
        Ok(Command::ShowTrace { archive, out, open })
    }
}

fn parse_bless_args(rest: &[String]) -> Result<Command, CliError> {
    {
        let mut dir = None;
        let mut rest: &[String] = rest;
        let mut dry_run = false;

        while let Some(flag) = rest.first() {
            match flag.as_str() {
                "--dry-run" => {
                    dry_run = true;
                    rest = &rest[1..];
                }
                other if dir.is_none() && !other.starts_with("--") => {
                    dir = Some(PathBuf::from(other));
                    rest = &rest[1..];
                }
                other => return Err(CliError::UnknownCommand(other.to_string())),
            }
        }
        let dir = dir.ok_or(CliError::Usage("bless needs a baseline directory"))?;
        Ok(Command::Bless { dir, dry_run })
    }
}

fn parse_install_deps_args(rest: &[String]) -> Result<Command, CliError> {
    {
        let mut run = false;
        for flag in rest {
            match flag.as_str() {
                "--run" => run = true,
                other => return Err(CliError::UnknownCommand(other.to_string())),
            }
        }
        Ok(Command::InstallDeps { run })
    }
}

/// `guiwright codegen [--uds <path> | --tcp <addr>] [--attach-ms N]
/// [--duration-ms N] [--out <path>] <app-cmd…>` — the launcher flags
/// precede the app command; once the first non-flag token appears, every
/// remaining token (flags included) belongs to the app (the `env`-wrapper
/// rule). A bare `--` ends the launcher flags explicitly, so an app
/// program whose name starts with `-` still launches (m4 §4).
fn parse_codegen_args(rest: &[String]) -> Result<Command, CliError> {
    let mut door = None;
    let mut attach_ms = None;
    let mut duration_ms = None;
    let mut out = None;
    let mut command: Option<Vec<String>> = None;
    let mut rest: &[String] = rest;
    while command.is_none() {
        let Some(flag) = rest.first() else {
            break;
        };
        match flag.as_str() {
            "--uds" | "--tcp" => {
                if door.is_some() {
                    return Err(CliError::Usage("--uds and --tcp are mutually exclusive"));
                }
                let (parsed, consumed) = parse_target_flag(flag, rest)?;
                door = Some(parsed);
                rest = &rest[consumed..];
            }
            "--attach-ms" | "--duration-ms" => {
                let (usage, slot): (&'static str, &mut Option<u64>) = if flag == "--attach-ms" {
                    ("--attach-ms needs milliseconds", &mut attach_ms)
                } else {
                    ("--duration-ms needs milliseconds", &mut duration_ms)
                };
                *slot = Some(parse_numeric_flag(rest, usage, "a deadline")?);
                rest = &rest[2..];
            }
            "--out" => {
                out = Some(PathBuf::from(flag_value(rest, "--out needs a path")?));
                rest = &rest[2..];
            }
            "--" => {
                command = Some(rest[1..].to_vec());
                rest = &[];
            }
            other if other.starts_with('-') => {
                return Err(CliError::UnknownCommand(other.to_string()));
            }
            _ => {
                // The app command: this token and everything after it.
                command = Some(rest.to_vec());
                rest = &[];
            }
        }
    }
    let command = command.filter(|c| !c.is_empty()).ok_or(CliError::Usage(
        "codegen needs an app command to launch: guiwright codegen <app-cmd…>",
    ))?;
    Ok(Command::Codegen {
        door,
        attach_ms,
        duration_ms,
        out,
        command,
    })
}

/// The value after a flag (`--workers 4`), or the usage error.
fn flag_value<'a>(rest: &'a [String], usage: &'static str) -> Result<&'a String, CliError> {
    rest.get(1).ok_or(CliError::Usage(usage))
}

/// `--uds <path>` / `--tcp <addr>` / `--ws <addr>`: the parsed target +
/// how many argv slots it consumed (2 — flag + value).
fn parse_target_flag(flag: &str, rest: &[String]) -> Result<(ConnectTarget, usize), CliError> {
    match flag {
        "--uds" => {
            let value = flag_value(rest, "--uds needs a socket path")?;
            Ok((ConnectTarget::Uds(PathBuf::from(value)), 2))
        }
        "--tcp" => {
            let value = flag_value(rest, "--tcp needs a host:port address")?;
            Ok((ConnectTarget::Tcp(value.clone()), 2))
        }
        "--ws" => {
            let value = flag_value(rest, "--ws needs a ws://host:port address")?;
            Ok((ConnectTarget::Ws(value.clone()), 2))
        }
        other => unreachable!("validated flag {other:?}"),
    }
}

/// Parse a numeric flag value with the shared "needs a {usage}" /
/// "not a {what}" error shape.
fn parse_numeric_flag<T: std::str::FromStr>(
    rest: &[String],
    usage: &'static str,
    what: &'static str,
) -> Result<T, CliError> {
    let value = flag_value(rest, usage)?;
    value
        .parse::<T>()
        .map_err(|_| CliError::BadValue(format!("`{value}` is not {what}")))
}

fn parse_test_args(rest: &[String]) -> Result<Command, CliError> {
    {
        let mut paths = Vec::new();
        let mut rest: &[String] = rest;
        let mut target: Option<ConnectTarget> = None;
        let mut workers: Option<usize> = None;
        let mut retries: Option<u32> = None;
        let mut timeout_ms: Option<u64> = None;
        let mut shard = None;
        let mut list = false;
        let mut quiet = false;
        let mut reports = Vec::new();
        let mut out: Option<PathBuf> = None;
        let mut trace = false;

        while let Some(flag) = rest.first() {
            match flag.as_str() {
                "--uds" | "--tcp" | "--ws" => {
                    if target.is_some() {
                        return Err(CliError::Usage(
                            "--uds, --tcp and --ws are mutually exclusive",
                        ));
                    }
                    let (parsed, consumed) = parse_target_flag(flag, rest)?;
                    target = Some(parsed);
                    rest = &rest[consumed..];
                }
                "--workers" => {
                    let value: usize =
                        parse_numeric_flag(rest, "--workers needs a count", "a worker count")?;
                    if value == 0 {
                        return Err(CliError::BadValue("--workers must be at least 1".into()));
                    }
                    workers = Some(value);
                    rest = &rest[2..];
                }
                "--retries" => {
                    retries = Some(parse_numeric_flag(
                        rest,
                        "--retries needs a count",
                        "a retry count",
                    )?);
                    rest = &rest[2..];
                }
                "--timeout" => {
                    timeout_ms = Some(parse_numeric_flag(
                        rest,
                        "--timeout needs milliseconds",
                        "a deadline",
                    )?);
                    rest = &rest[2..];
                }
                "--shard" => {
                    let value = flag_value(rest, "--shard needs k/n")?;
                    shard = Some(parse_shard(value)?);
                    rest = &rest[2..];
                }
                "--list" => {
                    list = true;
                    rest = &rest[1..];
                }
                "--quiet" | "-q" => {
                    quiet = true;
                    rest = &rest[1..];
                }
                "--report" => {
                    reports.push(parse_report_flag(rest)?);
                    rest = &rest[2..];
                }
                "--out" => {
                    out = Some(PathBuf::from(flag_value(rest, "--out needs a directory")?));
                    rest = &rest[2..];
                }
                "--trace" => {
                    trace = true;
                    rest = &rest[1..];
                }
                other if !other.starts_with('-') => {
                    paths.push(PathBuf::from(other));
                    rest = &rest[1..];
                }
                other => return Err(CliError::UnknownCommand(other.to_string())),
            }
        }
        Ok(Command::Test {
            paths,
            target,
            workers: workers.unwrap_or(1),
            retries,
            timeout_ms,
            shard,
            list,
            quiet,
            reports,
            out,
            trace,
        })
    }
}

/// `--report <kind>`: the value after the flag, parsed into its kind —
/// the usage error names the vocabulary, the bad-value error names what
/// was given.
fn parse_report_flag(rest: &[String]) -> Result<ReportKind, CliError> {
    let value = flag_value(rest, "--report needs list|json|junit|html")?;
    ReportKind::parse(value).ok_or_else(|| {
        CliError::BadValue(format!(
            "`{value}` is not a report kind — list | json | junit | html"
        ))
    })
}

/// `--shard k/n`: 1-indexed shard `k` of `n`.
fn parse_shard(value: &str) -> Result<(u32, u32), CliError> {
    let (k, n) = value
        .split_once('/')
        .ok_or_else(|| CliError::BadValue(format!("`{value}` is not k/n")))?;
    let k: u32 = k
        .parse()
        .map_err(|_| CliError::BadValue(format!("`{k}` is not a shard index")))?;
    let n: u32 = n
        .parse()
        .map_err(|_| CliError::BadValue(format!("`{n}` is not a shard total")))?;
    if n == 0 || k == 0 || k > n {
        return Err(CliError::BadValue(format!(
            "shard index must satisfy 1 <= k <= n, got {value}"
        )));
    }
    Ok((k, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn repl_parses_flag_and_bare_targets() {
        let cmd = parse_args(&args(&["repl", "--uds", "/tmp/a.sock"])).unwrap();
        assert_eq!(
            cmd,
            Command::Repl {
                target: ConnectTarget::Uds(PathBuf::from("/tmp/a.sock")),
                json: false,
            }
        );
        let cmd = parse_args(&args(&["repl", "--tcp", "127.0.0.1:9"])).unwrap();
        assert_eq!(
            cmd,
            Command::Repl {
                target: ConnectTarget::Tcp("127.0.0.1:9".into()),
                json: false,
            }
        );
        let cmd = parse_args(&args(&["repl", "uds:///x.sock"])).unwrap();
        assert_eq!(
            cmd,
            Command::Repl {
                target: ConnectTarget::Uds(PathBuf::from("/x.sock")),
                json: false,
            }
        );
        let cmd = parse_args(&args(&["repl", "--ws", "ws://127.0.0.1:9"])).unwrap();
        assert_eq!(
            cmd,
            Command::Repl {
                target: ConnectTarget::Ws("ws://127.0.0.1:9".into()),
                json: false,
            }
        );
        assert!(parse_args(&args(&["repl"])).is_err());
    }

    // The m11/m13 records' named CLI milestone (docs/m11-full-tree-snapshot.md
    // §5: "a `--json` output mode for machine consumers rides the CLI's
    // output-formatting milestones"): the flag rides the `repl` subcommand,
    // target required as ever, and the no-flag form stays the text mode.
    #[test]
    fn repl_json_flag_pins_the_machine_readable_mode() {
        let cmd = parse_args(&args(&["repl", "--uds", "/tmp/a.sock", "--json"])).unwrap();
        assert_eq!(
            cmd,
            Command::Repl {
                target: ConnectTarget::Uds(PathBuf::from("/tmp/a.sock")),
                json: true,
            }
        );
        let cmd = parse_args(&args(&["repl", "uds:///x.sock", "--json"])).unwrap();
        assert_eq!(
            cmd,
            Command::Repl {
                target: ConnectTarget::Uds(PathBuf::from("/x.sock")),
                json: true,
            }
        );
        let cmd = parse_args(&args(&["repl", "--uds", "/tmp/a.sock"])).unwrap();
        assert_eq!(
            cmd,
            Command::Repl {
                target: ConnectTarget::Uds(PathBuf::from("/tmp/a.sock")),
                json: false,
            }
        );
    }

    #[test]
    fn show_trace_parses_flags() {
        let cmd = parse_args(&args(&["show-trace", "a.zip", "--out", "b.html", "--open"])).unwrap();
        assert_eq!(
            cmd,
            Command::ShowTrace {
                archive: PathBuf::from("a.zip"),
                out: Some(PathBuf::from("b.html")),
                open: true,
            }
        );
        assert!(parse_args(&args(&["show-trace"])).is_err());
    }

    #[test]
    fn live_html_parses_target_and_flags() {
        let cmd = parse_args(&args(&[
            "live-html",
            "uds:///tmp/live.sock",
            "--out",
            "capture.html",
            "--duration-ms",
            "500",
            "--test",
            "live-demo",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            Command::LiveHtml {
                target: ConnectTarget::Uds(PathBuf::from("/tmp/live.sock")),
                out: Some(PathBuf::from("capture.html")),
                duration_ms: Some(500),
                test: Some("live-demo".into()),
            }
        );
        // The flagged target forms parse the same way the REPL's do.
        let cmd = parse_args(&args(&["live-html", "--tcp", "127.0.0.1:9000"])).unwrap();
        assert_eq!(
            cmd,
            Command::LiveHtml {
                target: ConnectTarget::Tcp("127.0.0.1:9000".into()),
                out: None,
                duration_ms: None,
                test: None,
            }
        );
        // No target, bad duration: the documented usage errors.
        assert!(parse_args(&args(&["live-html"])).is_err());
        assert!(parse_args(&args(&["live-html", "--duration-ms", "abc"])).is_err());
    }

    #[test]
    fn bless_and_install_deps_parse() {
        let cmd = parse_args(&args(&["bless", "shots", "--dry-run"])).unwrap();
        assert_eq!(
            cmd,
            Command::Bless {
                dir: PathBuf::from("shots"),
                dry_run: true,
            }
        );
        assert!(parse_args(&args(&["bless"])).is_err());
        let cmd = parse_args(&args(&["install-deps", "--run"])).unwrap();
        assert_eq!(cmd, Command::InstallDeps { run: true });
    }

    #[test]
    fn help_version_and_unknown() {
        assert!(matches!(
            parse_args(&args(&["--help"])),
            Err(CliError::Usage(_))
        ));
        assert!(matches!(
            parse_args(&args(&["--version"])),
            Err(CliError::Usage(_))
        ));
        assert!(matches!(
            parse_args(&args(&["wat"])),
            Err(CliError::UnknownCommand(_))
        ));
        assert!(parse_args(&args(&[])).is_err());
    }

    #[test]
    fn test_parses_full_flag_surface() {
        let cmd = parse_args(&args(&[
            "test",
            "tests/guiwright",
            "--uds",
            "/tmp/app.sock",
            "--workers",
            "4",
            "--retries",
            "2",
            "--timeout",
            "1500",
            "--shard",
            "2/3",
            "--list",
            "--quiet",
            "--report",
            "junit",
            "--report",
            "html",
            "--out",
            "ci-reports",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            Command::Test {
                paths: vec![PathBuf::from("tests/guiwright")],
                target: Some(ConnectTarget::Uds(PathBuf::from("/tmp/app.sock"))),
                workers: 4,
                retries: Some(2),
                timeout_ms: Some(1500),
                shard: Some((2, 3)),
                list: true,
                quiet: true,
                reports: vec![ReportKind::Junit, ReportKind::Html],
                out: Some(PathBuf::from("ci-reports")),
                trace: false,
            }
        );
    }

    #[test]
    fn test_defaults_and_bare_tcp_target() {
        let cmd = parse_args(&args(&["test", "--tcp", "127.0.0.1:41234"])).unwrap();
        assert_eq!(
            cmd,
            Command::Test {
                paths: Vec::new(),
                target: Some(ConnectTarget::Tcp("127.0.0.1:41234".into())),
                workers: 1,
                retries: None,
                timeout_ms: None,
                shard: None,
                list: false,
                quiet: false,
                reports: Vec::new(),
                out: None,
                trace: false,
            }
        );
    }

    #[test]
    fn test_rejects_bad_values() {
        // workers 0
        assert!(parse_args(&args(&["test", "--workers", "0"])).is_err());
        // non-numeric workers
        assert!(parse_args(&args(&["test", "--workers", "many"])).is_err());
        // bad shard forms
        assert!(parse_args(&args(&["test", "--shard", "2"])).is_err());
        assert!(parse_args(&args(&["test", "--shard", "0/2"])).is_err());
        assert!(parse_args(&args(&["test", "--shard", "3/2"])).is_err());
        assert!(parse_args(&args(&["test", "--shard", "a/b"])).is_err());
        // unknown report kind
        assert!(parse_args(&args(&["test", "--report", "xml"])).is_err());
        // flags missing their values
        assert!(parse_args(&args(&["test", "--uds"])).is_err());
        assert!(parse_args(&args(&["test", "--report"])).is_err());
        // mutually exclusive targets
        assert!(parse_args(&args(&["test", "--uds", "/a.sock", "--tcp", "h:1"])).is_err());
        // unknown flag
        assert!(parse_args(&args(&["test", "--bogus"])).is_err());
    }
}
