//! `guiwright install-deps` — the §11 CI provisioning printer.
//!
//! §11: "Guiwright CLI `install-deps` provisions CI (apt: libwayland,
//! xkbcommon, lavapipe/mesa; docs per matrix)." The command prints the
//! exact commands for the host OS; `--run` executes them (opt-in —
//! tests and docs never shell out).

use std::process::Command;

/// The provisioning commands for the host OS (§11 headless & CI table).
/// Empty = nothing known for this platform (macOS/Windows runners need
/// no extra system packages for the current adapter matrix).
pub fn platform_deps() -> Result<Vec<String>, String> {
    match std::env::consts::OS {
        "linux" => Ok(vec![
            "sudo apt-get update".into(),
            "sudo apt-get install -y libwayland-client0 libxkbcommon0 \
             mesa-vulkan-drivers libvulkan1 vulkan-tools"
                .into(),
        ]),
        // §11: macOS runners use Metal natively; Windows runners ship the
        // GPU stack — no system packages are provisioned today.
        "macos" | "windows" => Ok(Vec::new()),
        other => Err(format!(
            "no provisioning recipe for `{other}` — see docs/architecture.md §11"
        )),
    }
}

/// Print the commands (or run them with `run = true`). Returns whether
/// anything was printed/run.
pub fn install_deps_command(run: bool) -> Result<usize, String> {
    let commands = platform_deps()?;
    if commands.is_empty() {
        println!(
            "no extra system packages needed on {} (§11)",
            std::env::consts::OS
        );
        return Ok(0);
    }
    for command in &commands {
        println!("{command}");
    }
    if run {
        for command in &commands {
            // The §11 commands are a fixed table (not user input); sudo
            // prompts are the user's own terminal concern.
            let status = Command::new("sh")
                .arg("-c")
                .arg(command)
                .status()
                .map_err(|e| format!("`{command}` failed to launch: {e}"))?;
            if !status.success() {
                return Err(format!("`{command}` exited with {status}"));
            }
        }
    }
    Ok(commands.len())
}
