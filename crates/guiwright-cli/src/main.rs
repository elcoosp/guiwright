//! The `guiwright` binary (§5.2): thin dispatch over the library — every
//! behavior lives in the lib so tests and scripted use share one path.

use guiwright_cli::{Command, bless_dir, install_deps_command, parse_args, show_trace};

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&argv);
    std::process::exit(code);
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
        Command::Repl { target } => {
            // A current-thread runtime is plenty: the REPL is one
            // sequential conversation; the clients only need io + net.
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    eprintln!("error: tokio runtime: {err}");
                    return 1;
                }
            };
            rt.block_on(repl_loop(target))
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
            use std::io::Write;
            let _ = std::io::stderr().flush();
        }
        if outcome.exit {
            return 0;
        }
    }
    // EOF without `bye`: the session ends with the process.
    0
}
