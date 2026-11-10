//! `guiwright bless` — the §10 bless flow as an explicit command.
//!
//! The kit's `GUIWRIGHT_BLESS=1` flow leaves `<baseline>.new.png` and
//! `<baseline>.diff.png` artifacts next to a failing baseline. CI uploads
//! them, and a human blesses afterwards — on a machine where no test
//! runs. This command closes that loop: every
//! `<name>/<os>-<arch>-<renderer>.png.new.png` artifact under the
//! baseline root replaces its stale baseline, and the rendered diff is
//! removed with it.
//!
//! The layout is matched layout-based (§10
//! `<root>/<name>/<os>-<arch>-<renderer>.png`) so this command stays
//! independent of the shot crate's image stack — blessing moves files,
//! it never decodes pixels.

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BlessError {
    #[error("baseline directory {} does not exist", .0.display())]
    MissingDir(PathBuf),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// One promoted artifact (or the dry-run equivalent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlessReport {
    /// `guiwright-video`-style display path (relative to the root).
    pub name: String,
    /// The `.new.png` artifact that replaced the baseline.
    pub artifact: PathBuf,
    /// The rendered diff removed alongside (when one existed).
    pub diff_removed: bool,
    /// `false` for a `--dry-run` listing.
    pub applied: bool,
}

/// Bless every `.new.png` artifact under `root`.
///
/// * `dry_run` — list the promotions without touching anything.
///
/// Returns one report per promoted artifact, in stable (sorted) order.
pub fn bless_dir(root: &Path, dry_run: bool) -> Result<Vec<BlessReport>, BlessError> {
    if !root.is_dir() {
        return Err(BlessError::MissingDir(root.to_path_buf()));
    }
    // `<root>/<name>/*-<renderer>.png.new.png` — the §10 store shape, one
    // level of case directories.
    let mut reports = Vec::new();
    let mut cases: Vec<PathBuf> = std::fs::read_dir(root)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_dir())
        .collect();
    cases.sort();
    for case in cases {
        let mut artifacts: Vec<PathBuf> = std::fs::read_dir(&case)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| {
                path.file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with(".png.new.png"))
            })
            .collect();
        artifacts.sort();
        for artifact in artifacts {
            // Artifact names ride the §10 store shape:
            // `<os>-<arch>-<renderer>.png.new.png` (the `.new.png` sibling
            // of baseline `<os>-<arch>-<renderer>.png`, with the rendered
            // `.diff.png` next to both).
            let (baseline_name, diff_name) = match artifact.file_name().and_then(|n| n.to_str()) {
                Some(name) => match name.strip_suffix(".new.png") {
                    Some(base) => match base.strip_suffix(".png") {
                        Some(stem) => (base, format!("{stem}.diff.png")),
                        None => continue,
                    },
                    None => continue,
                },
                None => continue,
            };
            let baseline = case.join(baseline_name);
            let diff = case.join(diff_name);
            let name = format!(
                "{}/{}",
                case.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                baseline_name
            );
            let diff_removed = diff.exists();
            if !dry_run {
                std::fs::rename(&artifact, &baseline)?;
                if diff_removed {
                    let _ = std::fs::remove_file(&diff);
                }
            }
            reports.push(BlessReport {
                name,
                artifact,
                diff_removed,
                applied: !dry_run,
            });
        }
    }
    Ok(reports)
}
