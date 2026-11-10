//! `Guiwright.toml` — screenshot tolerance config (§10: "config in
//! `Guiwright.toml` with per-OS `[macos]`/`[windows]`/`[linux]` overrides;
//! we inherit kittest.toml's shape *and* its parser, avoiding the
//! documented `acos]` README bug").
//!
//! Shape (kittest.toml-compatible, namespaced under `[screenshots]`):
//!
//! ```toml
//! [screenshots]
//! threshold = 0.6          # weighted squared YIQ distance (§10)
//! max_failed_pixels = 0    # absolute pixel count
//!
//! [screenshots.linux]
//! threshold = 0.6
//! max_failed_pixels = 2
//! ```

use std::path::Path;

use serde::Deserialize;

use crate::ShotError;
use crate::diff::DEFAULT_THRESHOLD;

/// Top-level `Guiwright.toml` (only the screenshot section is defined at
/// M1; unknown keys are ignored so app configs stay forward-compatible).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct GuiwrightConfig {
    pub screenshots: ScreenshotsConfig,
}

/// Tolerances with per-OS overrides (kittest.toml shape, §10).
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ScreenshotsConfig {
    pub threshold: f32,
    pub max_failed_pixels: usize,
    pub linux: Option<OsTolerance>,
    pub macos: Option<OsTolerance>,
    pub windows: Option<OsTolerance>,
}

impl Default for ScreenshotsConfig {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_THRESHOLD,
            max_failed_pixels: 0,
            linux: None,
            macos: None,
            windows: None,
        }
    }
}

/// Per-OS tolerance override (missing fields fall back to the top level).
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct OsTolerance {
    pub threshold: Option<f32>,
    pub max_failed_pixels: Option<usize>,
}

/// The effective tolerances for the host OS (§10 "per-OS overrides").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectiveTolerance {
    pub threshold: f32,
    pub max_failed_pixels: usize,
}

impl ScreenshotsConfig {
    /// Resolve for the running OS (`std::env::consts::OS`: "linux",
    /// "macos", "windows"); unknown OSes get the top-level values.
    pub fn effective(&self) -> EffectiveTolerance {
        self.effective_for(std::env::consts::OS)
    }

    /// Resolve for an explicit OS token (testable without host coupling).
    pub fn effective_for(&self, os: &str) -> EffectiveTolerance {
        let over = match os {
            "linux" => &self.linux,
            "macos" => &self.macos,
            "windows" => &self.windows,
            _ => &None,
        };
        EffectiveTolerance {
            threshold: over.and_then(|o| o.threshold).unwrap_or(self.threshold),
            max_failed_pixels: over
                .and_then(|o| o.max_failed_pixels)
                .unwrap_or(self.max_failed_pixels),
        }
    }
}

/// Parse `Guiwright.toml` bytes.
pub fn parse_config(bytes: &str) -> Result<GuiwrightConfig, ShotError> {
    toml::from_str(bytes).map_err(|e| ShotError::Config(e.to_string()))
}

/// Load `Guiwright.toml` from a directory (missing file = defaults — a
/// project without visual tolerances gets the kittest defaults, §10).
pub fn load_from_dir(dir: &Path) -> Result<GuiwrightConfig, ShotError> {
    match std::fs::read_to_string(dir.join("Guiwright.toml")) {
        Ok(text) => parse_config(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(GuiwrightConfig::default()),
        Err(e) => Err(ShotError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_defaults() {
        let cfg = parse_config("").expect("empty table");
        let eff = cfg.screenshots.effective_for("linux");
        assert_eq!(eff.threshold, DEFAULT_THRESHOLD);
        assert_eq!(eff.max_failed_pixels, 0);
    }

    #[test]
    fn per_os_overrides_win() {
        let cfg = parse_config(
            r"
            [screenshots]
            threshold = 0.6
            max_failed_pixels = 0

            [screenshots.macos]
            threshold = 1.5
            max_failed_pixels = 8
            ",
        )
        .expect("parse");
        let mac = cfg.screenshots.effective_for("macos");
        assert_eq!((mac.threshold, mac.max_failed_pixels), (1.5, 8));
        let lin = cfg.screenshots.effective_for("linux");
        assert_eq!((lin.threshold, lin.max_failed_pixels), (0.6, 0));
        let win = cfg.screenshots.effective_for("windows");
        assert_eq!((win.threshold, win.max_failed_pixels), (0.6, 0));
    }

    #[test]
    fn override_fields_fall_back_independently() {
        let cfg = parse_config(
            r"
            [screenshots]
            threshold = 0.6

            [screenshots.windows]
            max_failed_pixels = 4
            ",
        )
        .expect("parse");
        let win = cfg.screenshots.effective_for("windows");
        assert_eq!((win.threshold, win.max_failed_pixels), (0.6, 4));
    }

    #[test]
    fn unknown_keys_and_sections_are_forward_compatible() {
        let cfg = parse_config(
            r#"
            [app]
            name = "demo"

            [screenshots]
            threshold = 0.6
            future_setting = "ignored"
            "#,
        )
        .expect("unknown keys must not break parsing");
        assert_eq!(cfg.screenshots.effective_for("linux").threshold, 0.6);
    }

    #[test]
    fn malformed_config_is_a_soft_error() {
        let err = parse_config("[screenshots\nthreshold = ").expect_err("toml error");
        assert!(err.to_string().contains("config"), "{err}");
    }
}
