//! Hand-rolled argument parsing (the workspace carries no clap): a
//! subcommand plus a handful of flags, fully unit-tested.

use std::path::PathBuf;

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
        other => Err(CliError::UnknownCommand(other.to_string())),
    }
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
}
