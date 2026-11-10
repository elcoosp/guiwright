//! Baseline storage + bless flow (§10):
//!
//! * Layout: `tests/screenshots/<name>/<os>-<arch>-<renderer>.png` —
//!   "Baselines stored per platform+scale+renderer" (iced's
//!   renderer-suffix lesson; the renderer token comes from the adapter so
//!   wgpu/tiny-skia/skia baselines never fight each other).
//! * `GUIWRIGHT_BLESS=1` updates **failing** baselines; `GUIWRIGHT_BLESS=force`
//!   updates **all** ("`force` updates all", §10).
//! * Failing runs emit `<name>.new.png` (actual) + `<name>.diff.png`
//!   (red-highlight diff) next to the baseline (§10).

use std::path::{Path, PathBuf};

use crate::codec::{decode_png, encode_png};
use crate::config::EffectiveTolerance;
use crate::diff::{diff, diff_image};
use crate::{Image, ShotError};

/// The bless mode, parsed from `GUIWRIGHT_BLESS` (§10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlessMode {
    /// Regular run: baselines are asserted, never written.
    #[default]
    Off,
    /// `GUIWRIGHT_BLESS=1` — write/replace baselines that fail.
    Failing,
    /// `GUIWRIGHT_BLESS=force` — write/replace every baseline.
    Force,
}

impl BlessMode {
    /// Parse the env value ("", "1", "true", "force"); unknown values are
    /// `Off` (documented, so `GUIWRIGHT_BLESS=treu` never silently blesses).
    pub fn parse(value: Option<&str>) -> Self {
        match value.unwrap_or("").trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => BlessMode::Failing,
            "force" => BlessMode::Force,
            _ => BlessMode::Off,
        }
    }

    /// Read `GUIWRIGHT_BLESS` from the process environment.
    pub fn from_env() -> Self {
        Self::parse(std::env::var("GUIWRIGHT_BLESS").ok().as_deref())
    }
}

/// What `assert_snapshot` decided (§10).
#[derive(Clone, Debug, PartialEq)]
pub enum SnapshotOutcome {
    /// Baseline matched within budget.
    Matched { failed_pixels: usize },
    /// Baseline written this run (fresh baseline or bless).
    Blessed { forced: bool },
    /// Baseline mismatched — artifacts were written next to it.
    Mismatched { report: String },
}

/// Root of the baseline tree (typically `<crate>/tests/screenshots`).
#[derive(Clone, Debug)]
pub struct BaselineStore {
    root: PathBuf,
    os: &'static str,
    arch: &'static str,
}

impl BaselineStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        }
    }

    /// Test-only constructor pinning the platform tokens (§10 layout is
    /// `<os>-<arch>-<renderer>.png`; tests must not depend on the host).
    pub fn for_platform(root: impl Into<PathBuf>, os: &'static str, arch: &'static str) -> Self {
        Self {
            root: root.into(),
            os,
            arch,
        }
    }

    /// `tests/screenshots/<name>/<os>-<arch>-<renderer>.png` (§10).
    pub fn baseline_path(&self, name: &str, renderer: &str) -> PathBuf {
        self.root.join(sanitize(name)).join(format!(
            "{}-{}-{}.png",
            self.os,
            self.arch,
            sanitize(renderer)
        ))
    }

    /// Where failing-run artifacts land (next to the baseline, §10).
    fn artifact_paths(&self, name: &str, renderer: &str) -> (PathBuf, PathBuf) {
        let base = self.baseline_path(name, renderer);
        let stem = base.file_stem().unwrap_or_default().to_string_lossy();
        let dir = base
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        (
            dir.join(format!("{stem}.new.png")),
            dir.join(format!("{stem}.diff.png")),
        )
    }

    /// The full §10 flow for one `expect(...).to_have_screenshot(name)`.
    ///
    /// * missing baseline: write it when blessing, else fail;
    /// * present baseline: diff within tolerance budget (§10);
    ///   mismatch → artifacts + error; bless → rewrite;
    /// * `BlessMode::Force` → always rewrite (and report `Blessed`).
    pub fn assert_snapshot(
        &self,
        name: &str,
        renderer: &str,
        actual: &Image,
        tolerance: EffectiveTolerance,
        bless: BlessMode,
    ) -> Result<SnapshotOutcome, ShotError> {
        let path = self.baseline_path(name, renderer);
        let baseline_bytes = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(ShotError::Io(e)),
        };

        let Some(bytes) = baseline_bytes else {
            if bless != BlessMode::Off {
                Self::write(&path, actual)?;
                return Ok(SnapshotOutcome::Blessed { forced: false });
            }
            return Err(ShotError::Config(format!(
                "missing baseline {} — run with GUIWRIGHT_BLESS=1 to create it",
                path.display()
            )));
        };

        let baseline = decode_png(&bytes)?;
        if !baseline.same_shape(actual) {
            return Err(ShotError::SizeMismatch {
                baseline_w: baseline.width,
                baseline_h: baseline.height,
                actual_w: actual.width,
                actual_h: actual.height,
            });
        }

        let report = diff(&baseline, actual, tolerance.threshold)
            .expect("shape checked above — diff cannot be None");

        if bless == BlessMode::Force {
            Self::write(&path, actual)?;
            return Ok(SnapshotOutcome::Blessed { forced: true });
        }

        if report.passes(tolerance.max_failed_pixels) {
            return Ok(SnapshotOutcome::Matched {
                failed_pixels: report.failed_pixels,
            });
        }

        // Failing run: emit `.new.png` + `.diff.png` (§10), then fail.
        let (new_path, diff_path) = self.artifact_paths(name, renderer);
        Self::write(&new_path, actual)?;
        let diff_render = diff_image(&baseline, &report, tolerance.threshold);
        Self::write(&diff_path, &diff_render)?;
        if bless == BlessMode::Failing {
            Self::write(&path, actual)?;
            return Ok(SnapshotOutcome::Blessed { forced: false });
        }
        Err(ShotError::Config(format!(
            "screenshot `{name}` mismatch: {} — artifacts: {} + {}",
            report.summarize(tolerance.threshold, tolerance.max_failed_pixels),
            new_path.display(),
            diff_path.display()
        )))
    }

    fn write(path: &Path, image: &Image) -> Result<(), ShotError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, encode_png(image)?)?;
        Ok(())
    }
}

/// Path component sanitizer: names and renderer tokens must not escape the
/// baseline directory and must be stable across filesystems. Non-alphanum
/// runs collapse to a single dash ("wgpu (lavapipe)" → "wgpu-lavapipe").
fn sanitize(component: &str) -> String {
    let mut out = String::with_capacity(component.len());
    let mut last_dash = true; // also trims leading dashes
    for c in component.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_end_matches('-').to_string();
    if trimmed.is_empty() {
        "unnamed".into()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ScreenshotsConfig;
    use std::fs;

    fn store(tmp: &Path) -> BaselineStore {
        BaselineStore::for_platform(tmp.join("screenshots"), "linux", "x86_64")
    }

    fn tolerance() -> EffectiveTolerance {
        ScreenshotsConfig::default().effective_for("linux")
    }

    #[test]
    fn bless_creates_missing_baseline() {
        let tmp = tempfile_dir();
        let st = store(&tmp);
        let img = Image::solid(4, 4, [1, 2, 3, 255]);
        let out = st
            .assert_snapshot(
                "login/empty",
                "swrast",
                &img,
                tolerance(),
                BlessMode::Failing,
            )
            .expect("bless");
        assert_eq!(out, SnapshotOutcome::Blessed { forced: false });
        assert!(st.baseline_path("login/empty", "swrast").exists());
    }

    #[test]
    fn missing_baseline_without_bless_fails_with_guidance() {
        let tmp = tempfile_dir();
        let st = store(&tmp);
        let err = st
            .assert_snapshot(
                "fresh",
                "swrast",
                &Image::solid(2, 2, [0, 0, 0, 255]),
                tolerance(),
                BlessMode::Off,
            )
            .expect_err("missing baseline");
        assert!(err.to_string().contains("GUIWRIGHT_BLESS=1"), "{err}");
    }

    #[test]
    fn matching_baseline_passes_and_writes_nothing() {
        let tmp = tempfile_dir();
        let st = store(&tmp);
        let img = Image::solid(4, 4, [9, 9, 9, 255]);
        st.assert_snapshot("m", "swrast", &img, tolerance(), BlessMode::Failing)
            .expect("seed baseline");
        let out = st
            .assert_snapshot("m", "swrast", &img, tolerance(), BlessMode::Off)
            .expect("match");
        assert_eq!(out, SnapshotOutcome::Matched { failed_pixels: 0 });
        // No artifacts leaked on success.
        assert!(
            !tmp.join("screenshots/m/linux-x86_64-swrast.new.png")
                .exists()
        );
        assert!(
            !tmp.join("screenshots/m/linux-x86_64-swrast.diff.png")
                .exists()
        );
    }

    #[test]
    fn mismatch_fails_and_emits_new_and_diff_artifacts() {
        let tmp = tempfile_dir();
        let st = store(&tmp);
        st.assert_snapshot(
            "dash",
            "tiny-skia",
            &Image::solid(8, 8, [0, 0, 0, 255]),
            tolerance(),
            BlessMode::Failing,
        )
        .expect("seed");
        let mut actual = Image::solid(8, 8, [0, 0, 0, 255]);
        actual.rgba[0..4].copy_from_slice(&[255, 255, 255, 255]);
        let err = st
            .assert_snapshot("dash", "tiny-skia", &actual, tolerance(), BlessMode::Off)
            .expect_err("mismatch");
        let msg = err.to_string();
        assert!(msg.contains("pixels differ"), "{msg}");
        assert!(
            tmp.join("screenshots/dash/linux-x86_64-tiny-skia.new.png")
                .exists()
        );
        assert!(
            tmp.join("screenshots/dash/linux-x86_64-tiny-skia.diff.png")
                .exists()
        );
    }

    #[test]
    fn bless_failing_rewrites_only_on_mismatch() {
        let tmp = tempfile_dir();
        let st = store(&tmp);
        let original = Image::solid(4, 4, [5, 5, 5, 255]);
        st.assert_snapshot("b", "r", &original, tolerance(), BlessMode::Failing)
            .expect("seed");
        // Passing snapshot + GUIWRIGHT_BLESS=1 → untouched (only *failing*
        // baselines update, kittest UPDATE_SNAPSHOTS=true semantics).
        let out = st
            .assert_snapshot("b", "r", &original, tolerance(), BlessMode::Failing)
            .expect("pass");
        assert_eq!(out, SnapshotOutcome::Matched { failed_pixels: 0 });
        // Mismatch + GUIWRIGHT_BLESS=1 → rewritten with the new pixels.
        let changed = Image::solid(4, 4, [250, 250, 250, 255]);
        let out = st
            .assert_snapshot("b", "r", &changed, tolerance(), BlessMode::Failing)
            .expect("bless on failure");
        assert_eq!(out, SnapshotOutcome::Blessed { forced: false });
        let reloaded =
            decode_png(&fs::read(st.baseline_path("b", "r")).expect("read")).expect("png");
        assert_eq!(reloaded, changed);
    }

    #[test]
    fn bless_force_rewrites_even_passing_baselines() {
        let tmp = tempfile_dir();
        let st = store(&tmp);
        let a = Image::solid(4, 4, [1, 1, 1, 255]);
        let b = Image::solid(4, 4, [2, 2, 2, 255]); // within threshold of a
        st.assert_snapshot("f", "r", &a, tolerance(), BlessMode::Failing)
            .expect("seed");
        let out = st
            .assert_snapshot("f", "r", &b, tolerance(), BlessMode::Force)
            .expect("force bless");
        assert_eq!(out, SnapshotOutcome::Blessed { forced: true });
        let reloaded =
            decode_png(&fs::read(st.baseline_path("f", "r")).expect("read")).expect("png");
        assert_eq!(reloaded, b);
    }

    #[test]
    fn size_mismatch_is_reported_not_panic() {
        let tmp = tempfile_dir();
        let st = store(&tmp);
        st.assert_snapshot(
            "s",
            "r",
            &Image::solid(4, 4, [0; 4]),
            tolerance(),
            BlessMode::Failing,
        )
        .expect("seed");
        let err = st
            .assert_snapshot(
                "s",
                "r",
                &Image::solid(2, 2, [0; 4]),
                tolerance(),
                BlessMode::Off,
            )
            .expect_err("size mismatch");
        assert!(matches!(err, ShotError::SizeMismatch { .. }), "{err}");
    }

    #[test]
    fn renderer_token_is_sanitized_into_the_path() {
        let tmp = tempfile_dir();
        let st = store(&tmp);
        let path = st.baseline_path("Main Window/2", "wgpu (lavapipe)");
        let rendered = path.to_string_lossy().to_string();
        assert!(
            rendered.contains("main-window-2/linux-x86_64-wgpu-lavapipe.png"),
            "{rendered}"
        );
    }

    #[test]
    fn bless_mode_parsing_is_strict() {
        assert_eq!(BlessMode::parse(Some("1")), BlessMode::Failing);
        assert_eq!(BlessMode::parse(Some("TRUE")), BlessMode::Failing);
        assert_eq!(BlessMode::parse(Some("force")), BlessMode::Force);
        assert_eq!(BlessMode::parse(Some("FORCE")), BlessMode::Force);
        assert_eq!(BlessMode::parse(Some("")), BlessMode::Off);
        assert_eq!(BlessMode::parse(Some("1 ")), BlessMode::Failing);
        assert_eq!(
            BlessMode::parse(Some("typo")),
            BlessMode::Off,
            "unknown values never bless"
        );
        assert_eq!(BlessMode::parse(None), BlessMode::Off);
    }

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gw-shot-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("tmp dir");
        dir
    }
}
