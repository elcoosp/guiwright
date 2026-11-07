//! Hand-rolled argument parsing (the workspace carries no clap): a
//! subcommand plus a handful of flags, fully unit-tested.

use std::path::PathBuf;

use crate::report::ReportKind;
use crate::session::ConnectTarget;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// `guiwright repl --uds <path> | --tcp <addr>` — the §13 live session
    /// debugger.
    Repl { target: ConnectTarget },
    /// `guiwright show-trace <archive.zip> [--out <path>] [--open]` — the
    /// §12 deep-link target.
    ShowTrace {
        archive: PathBuf,
        out: Option<PathBuf>,
        open: bool,
    },
    /// `guiwright bless <dir> [--dry-run]` — the §10 bless flow as a CLI.
    Bless { dir: PathBuf, dry_run: bool },
    /// `guiwright install-deps [--run]` — the §11 CI provisioning printer.
    InstallDeps { run: bool },
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
                                  (bare paths and host:port also work)
    show-trace <archive>  Render a gwtrace/v1 zip into trace.html (§12).
        [--out <path>]    Output path (default: next to the archive).
        [--open]          Open the rendered file with the platform opener.
    bless <dir>           Promote .new.png artifacts onto their baselines
        [--dry-run]       (§10 GUIWRIGHT_BLESS flow, for CI artifacts).
    install-deps          Print the §11 CI provisioning commands for the
        [--run]           host OS (--run executes them).
    test [PATHS…]         Run gwtest/v1 scenario specs against a live app
                          (the §16 v0.5 own runner; default PATH:
                          tests/guiwright, scanned for *.spec.toml).
        [--uds <path> | --tcp <addr>]   Target override (beats spec files).
        [--workers N]     Parallel scenario workers (default 1).
        [--retries N]     Re-run failed scenarios (default: spec value).
        [--timeout <ms>]  Step deadline override for the whole run.
        [--shard k/n]     Deterministic interleaved split (§11, one app
                          per shard).
        [--list]          Print the discovered scenarios and exit.
        [--report <kind>] list | json | junit | html (repeatable; files
                          written under --out).
        [--out <dir>]     Report directory (default guiwright-report/).
        [-q | --quiet]    Suppress per-scenario progress lines.
    --help | --version

Run `guiwright repl` and type `help` for the in-session command list."
}

/// Parse `std::env::args().skip(1)`-style arguments.
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
        "repl" => {
            let mut target = None;
            let mut rest: &[String] = &argv[1..];
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
                    other => {
                        // Bare target: uds:///…, tcp://…, /path or host:port.
                        target = Some(ConnectTarget::parse(other).map_err(CliError::BadValue)?);
                        rest = &rest[1..];
                    }
                }
            }
            let target = target.ok_or(CliError::Usage(
                "repl needs a target: --uds <path> | --tcp <addr> | <target>",
            ))?;
            Ok(Command::Repl { target })
        }
        "show-trace" => {
            let mut archive = None;
            let mut out = None;
            let mut open = false;
            let mut rest: &[String] = &argv[1..];
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
        "bless" => {
            let mut dir = None;
            let mut dry_run = false;
            let mut rest: &[String] = &argv[1..];
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
        "install-deps" => {
            let mut run = false;
            for flag in &argv[1..] {
                match flag.as_str() {
                    "--run" => run = true,
                    other => return Err(CliError::UnknownCommand(other.to_string())),
                }
            }
            Ok(Command::InstallDeps { run })
        }
        "test" => {
            let mut paths = Vec::new();
            let mut target: Option<ConnectTarget> = None;
            let mut workers: Option<usize> = None;
            let mut retries: Option<u32> = None;
            let mut timeout_ms: Option<u64> = None;
            let mut shard = None;
            let mut list = false;
            let mut quiet = false;
            let mut reports = Vec::new();
            let mut out: Option<PathBuf> = None;
            let mut rest: &[String] = &argv[1..];
            while let Some(flag) = rest.first() {
                match flag.as_str() {
                    "--uds" | "--tcp" => {
                        if target.is_some() {
                            return Err(CliError::Usage(
                                "--uds and --tcp are mutually exclusive",
                            ));
                        }
                        let value = rest.get(1).ok_or(CliError::Usage(match flag.as_str() {
                            "--uds" => "--uds needs a socket path",
                            _ => "--tcp needs a host:port address",
                        }))?;
                        target = Some(match flag.as_str() {
                            "--uds" => ConnectTarget::Uds(PathBuf::from(value)),
                            _ => ConnectTarget::Tcp(value.clone()),
                        });
                        rest = &rest[2..];
                    }
                    "--workers" => {
                        let value = rest
                            .get(1)
                            .ok_or(CliError::Usage("--workers needs a count"))?
                            .parse::<usize>()
                            .map_err(|_| CliError::BadValue(format!("`{}` is not a worker count", rest[1])))?;
                        if value == 0 {
                            return Err(CliError::BadValue("--workers must be at least 1".into()));
                        }
                        workers = Some(value);
                        rest = &rest[2..];
                    }
                    "--retries" => {
                        let value = rest
                            .get(1)
                            .ok_or(CliError::Usage("--retries needs a count"))?
                            .parse::<u32>()
                            .map_err(|_| CliError::BadValue(format!("`{}` is not a retry count", rest[1])))?;
                        retries = Some(value);
                        rest = &rest[2..];
                    }
                    "--timeout" => {
                        let value = rest
                            .get(1)
                            .ok_or(CliError::Usage("--timeout needs milliseconds"))?
                            .parse::<u64>()
                            .map_err(|_| CliError::BadValue(format!("`{}` is not a deadline", rest[1])))?;
                        timeout_ms = Some(value);
                        rest = &rest[2..];
                    }
                    "--shard" => {
                        let value = rest
                            .get(1)
                            .ok_or(CliError::Usage("--shard needs k/n"))?;
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
                        let value = rest
                            .get(1)
                            .ok_or(CliError::Usage("--report needs list|json|junit|html"))?;
                        let kind = ReportKind::parse(value).ok_or_else(|| {
                            CliError::BadValue(format!(
                                "`{value}` is not a report kind — list | json | junit | html"
                            ))
                        })?;
                        reports.push(kind);
                        rest = &rest[2..];
                    }
                    "--out" => {
                        let value = rest.get(1).ok_or(CliError::Usage("--out needs a directory"))?;
                        out = Some(PathBuf::from(value));
                        rest = &rest[2..];
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
            })
        }
        other => Err(CliError::UnknownCommand(other.to_string())),
    }
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
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn repl_parses_flag_and_bare_targets() {
        let cmd = parse_args(&args(&["repl", "--uds", "/tmp/a.sock"])).unwrap();
        assert_eq!(
            cmd,
            Command::Repl {
                target: ConnectTarget::Uds(PathBuf::from("/tmp/a.sock"))
            }
        );
        let cmd = parse_args(&args(&["repl", "--tcp", "127.0.0.1:9"])).unwrap();
        assert_eq!(
            cmd,
            Command::Repl {
                target: ConnectTarget::Tcp("127.0.0.1:9".into())
            }
        );
        let cmd = parse_args(&args(&["repl", "uds:///x.sock"])).unwrap();
        assert_eq!(
            cmd,
            Command::Repl {
                target: ConnectTarget::Uds(PathBuf::from("/x.sock"))
            }
        );
        assert!(parse_args(&args(&["repl"])).is_err());
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
        assert!(
            parse_args(&args(&["test", "--uds", "/a.sock", "--tcp", "h:1"])).is_err()
        );
        // unknown flag
        assert!(parse_args(&args(&["test", "--bogus"])).is_err());
    }
}
