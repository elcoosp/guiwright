//! `guiwright show-trace` — the §12 deep-link target: render a
//! `gwtrace/v1` zip archive into the viewer's self-contained
//! `trace.html` (timeline scrubber, tree inspector, event/log views —
//! no server, no external assets).

use std::path::{Path, PathBuf};

use guiwright_viewer::{Timeline, TraceArchive, render_html};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ShowTraceError {
    #[error("viewer: {0}")]
    Viewer(#[from] guiwright_viewer::ViewerError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot determine an output path for {0}")]
    OutputPath(PathBuf),
}

/// Render `archive` into a self-contained `trace.html`.
///
/// * `out` — explicit output path; default `<archive-stem>.html` next to
///   the archive.
/// * `open` — hand the rendered file to the platform opener
///   (`xdg-open`/`open`/`start`); best-effort, a missing opener degrades
///   to printing the path.
///
/// Returns the output path (also printed to the user).
pub fn show_trace(
    archive: &Path,
    out: Option<PathBuf>,
    open: bool,
) -> Result<PathBuf, ShowTraceError> {
    let out = if let Some(path) = out {
        path
    } else {
        let stem = archive
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .ok_or_else(|| ShowTraceError::OutputPath(archive.to_path_buf()))?;
        archive
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
            .join(format!("{stem}.html"))
    };

    let archive_reader = TraceArchive::open(archive)?;
    let timeline = Timeline::from_archive(&archive_reader);
    let html = render_html(&archive_reader, &timeline);
    std::fs::write(&out, html)?;

    if open {
        open_in_browser(&out);
    }
    Ok(out)
}

/// Best-effort platform opener; failure is fine (the path is printed).
fn open_in_browser(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "windows")]
    let opener = "cmd";
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let opener = "xdg-open";

    #[cfg(target_os = "windows")]
    let status = std::process::Command::new(opener)
        .args(["/C", "start", "", &path.display().to_string()])
        .status();
    #[cfg(not(target_os = "windows"))]
    let status = std::process::Command::new(opener).arg(path).status();

    matches!(status, Ok(s) if s.success())
}
