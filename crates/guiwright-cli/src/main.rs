//! The `guiwright` binary (§5.2): thin dispatch over the library — every
//! behavior lives in the lib so tests and scripted use share one path.

use std::io::Write;

use guiwright_cli::{Command, bless_dir, install_deps_command, parse_args, show_trace};

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&argv);
    std::process::exit(code);
}

/// One current-thread tokio runtime (io + net) — the CLI is sequential.
fn current_thread_runtime() -> Result<tokio::runtime::Runtime, std::io::Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

fn run(argv: &[String]) -> i32 {
    // `--help` / `--version` are successful outputs, not usage errors —
    // stdout + exit 0 (CLI convention).
    match argv.first().map(String::as_str) {
        Some("--help" | "-h" | "help") => {
            println!("{}", guiwright_cli::help_text());
            return 0;
        }
        Some("--version" | "-V") => {
            println!("guiwright {}", env!("CARGO_PKG_VERSION"));
            return 0;
        }
        _ => {}
    }
    let command = match parse_args(argv) {
        Ok(command) => command,
        Err(err) => {
            eprintln!("{err}");
            return 2;
        }
    };
    match command {
        Command::Repl { target } => run_repl(target),
        Command::Test {
            paths,
            target,
            workers,
            retries,
            timeout_ms,
            shard,
            list,
            quiet,
            reports,
            out,
        } => {
            // The runner is IO-bound (one GWP connection per attempt);
            // a current-thread runtime interleaves the workers on the
            // await points.
            let rt = match current_thread_runtime() {
                Ok(rt) => rt,
                Err(err) => {
                    eprintln!("error: tokio runtime: {err}");
                    return 1;
                }
            };
            rt.block_on(test_loop(
                guiwright_cli::runner::TestOptions {
                    target,
                    workers,
                    retries,
                    timeout_ms,
                    shard,
                    list,
                    quiet,
                    reports,
                    out,
                },
                paths,
            ))
        }
        Command::ShowTrace { archive, out, open } => match show_trace(&archive, out, open) {
            Ok(path) => {
                println!("{}", path.display());
                0
            }
            Err(err) => {
                eprintln!("error: {err}");
                1
            }
        },
        Command::Bless { dir, dry_run } => match bless_dir(&dir, dry_run) {
            Ok(reports) => {
                if reports.is_empty() {
                    println!(
                        "nothing to bless — no .new.png artifacts under {}",
                        dir.display()
                    );
                }
                for report in &reports {
                    let action = if report.applied {
                        "blessed"
                    } else {
                        "would bless"
                    };
                    let diff = if report.diff_removed {
                        " (diff removed)"
                    } else {
                        ""
                    };
                    println!("{action}: {}{diff}", report.name);
                }
                0
            }
            Err(err) => {
                eprintln!("error: {err}");
                1
            }
        },
        Command::InstallDeps { run } => match install_deps_command(run) {
            Ok(_) => 0,
            Err(err) => {
                eprintln!("error: {err}");
                1
            }
        },
    }
}

/// The interactive front-end: read lines, feed the core, print the
/// outputs. The prompt goes to stderr (stdout stays parseable), and only
/// when stdin is a TTY — `echo 'snapshot' | guiwright repl …` is clean.
async fn repl_loop(target: guiwright_cli::ConnectTarget) -> i32 {
    use std::io::BufRead;
    use std::io::IsTerminal;

    let mut repl = match guiwright_cli::Repl::connect(&target).await {
        Ok(repl) => repl,
        Err(err) => {
            eprintln!("error: cannot attach to {target:?}: {err}");
            return 1;
        }
    };
    let meta = repl.adapter();
    eprintln!(
        "guiwright repl — {} adapter `{}` (framework `{}`); type `help` for commands",
        if matches!(target, guiwright_cli::ConnectTarget::Uds(_)) {
            "uds"
        } else {
            "tcp"
        },
        meta.name,
        meta.framework
    );

    let stdin = std::io::stdin();
    let interactive = stdin.is_terminal();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                eprintln!("error: stdin: {err}");
                break;
            }
        };
        let outcome = repl.step(&line).await;
        for line in &outcome.lines {
            match line {
                guiwright_cli::Line::Err(_) => eprintln!("{}", line.render()),
                other => println!("{}", other.render()),
            }
        }
        if interactive && !outcome.exit {
            eprint!("gw> ");
            let _ = std::io::stderr().flush();
        }
        if outcome.exit {
            return 0;
        }
    }
    // EOF without `bye`: the session ends with the process.
    0
}

/// The `guiwright repl` front-end: one current-thread runtime, one
/// sequential conversation (§16 REPL parity).
fn run_repl(target: guiwright_cli::session::ConnectTarget) -> i32 {
    match current_thread_runtime() {
        Ok(rt) => rt.block_on(repl_loop(target)),
        Err(err) => {
            eprintln!("error: tokio runtime: {err}");
            1
        }
    }
}

/// The `guiwright test` front-end (M5): discover → run → print → report.
/// Exit codes: 0 all passed (flaky included), 1 any failure, 2 config
/// errors (no specs, bad specs, missing target).
async fn test_loop(
    mut opts: guiwright_cli::runner::TestOptions,
    paths: Vec<std::path::PathBuf>,
) -> i32 {
    use guiwright_cli::report::{progress_line, summary_line};

    let mut roots = paths;
    if roots.is_empty() {
        roots.push(std::path::PathBuf::from(
            guiwright_cli::runner::DEFAULT_SPEC_DIR,
        ));
    }
    let specs = match guiwright_cli::runner::discover(&roots) {
        Ok(specs) => specs,
        Err(err) => {
            eprintln!("error: {err}");
            return 2;
        }
    };
    let on_finish = if opts.quiet {
        None
    } else {
        Some(
            std::sync::Arc::new(|record: &guiwright_cli::report::ScenarioRecord| {
                println!("{}", progress_line(record));
            })
                as std::sync::Arc<dyn Fn(&guiwright_cli::report::ScenarioRecord) + Send + Sync>,
        )
    };
    let outcome = match guiwright_cli::runner::run(specs, &opts, on_finish).await {
        Ok(outcome) => outcome,
        Err(err) => {
            eprintln!("error: {err}");
            return 2;
        }
    };
    if let Some(listed) = &outcome.listed {
        for id in listed {
            println!("{id}");
        }
        return 0;
    }
    if !opts.quiet {
        println!("{}", summary_line(&outcome.report));
    }
    // File reports (repeatable --report kinds; the default dir keeps the
    // no-flag invocation useful).
    if !opts.reports.is_empty() {
        let out_dir = opts
            .out
            .take()
            .unwrap_or_else(|| std::path::PathBuf::from("guiwright-report"));
        match guiwright_cli::runner::write_reports(&outcome.report, &opts.reports, &out_dir) {
            Ok(written) => {
                for path in written {
                    println!("report: {}", path.display());
                }
            }
            Err(err) => {
                eprintln!("error: writing reports: {err}");
                return 1;
            }
        }
    }
    i32::from(outcome.failed)
}
