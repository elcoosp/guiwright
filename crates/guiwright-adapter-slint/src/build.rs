//! The testing build helper (§4.4 "testing backend + build helper" and
//! §11 slint row: `SLINT_BACKEND=headless[-software|-skia]` +
//! `SLINT_EMIT_DEBUG_INFO=1`).
//!
//! Apps wire it in `build.rs`:
//!
//! ```ignore
//! fn main() {
//!     slint_build::compile("ui/app.slint").unwrap();
//!     for line in guiwright_adapter_slint::build::cargo_instructions() {
//!         println!("{line}");
//!     }
//! }
//! ```
//!
//! The emitted instructions (1) select the headless software backend so
//! CI runs without a display server, (2) enable debug info so element
//! *names* land in the testing backend's introspection — the natural
//! test ids (§8.1).

/// The testing-build configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TestingBuild {
    /// `SLINT_BACKEND` value (§11: headless software for CI determinism).
    pub backend: &'static str,
    /// `SLINT_EMIT_DEBUG_INFO` (element names in introspection).
    pub emit_debug_info: bool,
}

impl Default for TestingBuild {
    fn default() -> Self {
        Self {
            backend: "headless-software",
            emit_debug_info: true,
        }
    }
}

impl TestingBuild {
    /// The `cargo:` instructions a build.rs must print.
    pub fn cargo_instructions(&self) -> Vec<String> {
        let mut out = vec![format!("cargo:rustc-env=SLINT_BACKEND={}", self.backend)];
        if self.emit_debug_info {
            out.push("cargo:rustc-env=SLINT_EMIT_DEBUG_INFO=1".to_string());
        }
        out
    }

    /// The runtime env values, for apps that set them programmatically
    /// (tests, examples).
    pub fn env_values(&self) -> Vec<(&'static str, String)> {
        let mut out = vec![("SLINT_BACKEND", self.backend.to_string())];
        if self.emit_debug_info {
            out.push(("SLINT_EMIT_DEBUG_INFO", "1".to_string()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_build_emits_backend_and_debug_info() {
        let build = TestingBuild::default();
        let instructions = build.cargo_instructions();
        assert!(instructions.contains(&"cargo:rustc-env=SLINT_BACKEND=headless-software".into()));
        assert!(instructions.contains(&"cargo:rustc-env=SLINT_EMIT_DEBUG_INFO=1".into()));
        assert_eq!(instructions.len(), 2);
    }

    #[test]
    fn env_values_mirror_instructions() {
        let build = TestingBuild::default();
        let env: std::collections::HashMap<&str, String> = build.env_values().into_iter().collect();
        assert_eq!(
            env.get("SLINT_BACKEND").map(String::as_str),
            Some("headless-software")
        );
        assert_eq!(
            env.get("SLINT_EMIT_DEBUG_INFO").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn debug_info_can_be_disabled() {
        let build = TestingBuild {
            emit_debug_info: false,
            ..Default::default()
        };
        assert!(
            !build
                .cargo_instructions()
                .iter()
                .any(|l| l.contains("SLINT_EMIT_DEBUG_INFO"))
        );
    }
}
