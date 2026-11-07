//! `guiwright install-deps` (§11): the emitted provisioning commands name
//! the packages the CI matrix installs (libwayland, xkbcommon,
//! lavapipe/mesa + the Vulkan loader/tools on Linux).

use guiwright_cli::platform_deps;

#[test]
fn linux_recipe_names_the_ci_packages() {
    // The recipe is keyed by the host OS; on a Linux runner (CI's test
    // platform) it names the §11 package set.
    if std::env::consts::OS != "linux" {
        return;
    }
    let commands = platform_deps().expect("linux recipe");
    assert!(!commands.is_empty());
    let joined = commands.join("\n");
    for package in [
        "libwayland-client0",
        "libxkbcommon0",
        "mesa-vulkan-drivers",
        "libvulkan1",
        "vulkan-tools",
    ] {
        assert!(joined.contains(package), "{joined}");
    }
}

#[test]
fn desktop_platforms_need_no_packages() {
    match std::env::consts::OS {
        "macos" | "windows" => {
            let commands = platform_deps().expect("no-op recipe");
            assert!(commands.is_empty(), "{commands:?}");
        }
        _ => {}
    }
}

#[test]
fn unknown_platforms_error_cleanly() {
    // The error path is platform-keyed; exercise it directly through the
    // same match the command uses (no platform override available).
    let known = ["linux", "macos", "windows"];
    if known.contains(&std::env::consts::OS) {
        return;
    }
    assert!(platform_deps().is_err());
}
