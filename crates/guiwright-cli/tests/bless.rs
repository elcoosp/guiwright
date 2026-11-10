//! `guiwright bless` (§10): the `.new.png` → baseline promotion flow as a
//! CLI, layout-based (no image decoding), with a dry-run.

use std::fs;

use guiwright_cli::bless_dir;

fn write(path: impl AsRef<std::path::Path>, bytes: impl AsRef<[u8]>) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn bless_promotes_artifacts_and_removes_diffs() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = dir.path();

    // The §10 store shape: <root>/<name>/<os>-<arch>-<renderer>.png plus
    // failing-run artifacts.
    write(
        root.join("login/linux-x86_64-default.png.new.png"),
        b"new-frame",
    );
    write(root.join("login/linux-x86_64-default.diff.png"), b"diff");
    write(
        root.join("settings/linux-x86_64-default.png.new.png"),
        b"new-frame-2",
    );
    // An untouched baseline must survive untouched.
    write(root.join("settings/macos-aarch64-metal.png"), b"baseline");
    // Unrelated files are ignored.
    write(root.join("settings/notes.txt"), b"keep me");

    let reports = bless_dir(root, false).expect("blessed");
    assert_eq!(reports.len(), 2, "{reports:?}");
    assert!(reports.iter().all(|r| r.applied));

    // Promoted: the artifact replaced the baseline.
    let login_baseline = root.join("login/linux-x86_64-default.png");
    assert_eq!(fs::read(&login_baseline).unwrap(), b"new-frame".as_slice());
    assert!(!root.join("login/linux-x86_64-default.png.new.png").exists());
    assert!(!root.join("login/linux-x86_64-default.diff.png").exists());

    let settings_baseline = root.join("settings/linux-x86_64-default.png");
    assert_eq!(
        fs::read(&settings_baseline).unwrap(),
        b"new-frame-2".as_slice()
    );
    assert!(
        !root
            .join("settings/linux-x86_64-default.png.new.png")
            .exists()
    );
    // No diff existed for this one — the report says so.
    let settings = reports
        .iter()
        .find(|r| r.name == "settings/linux-x86_64-default.png")
        .unwrap();
    assert!(!settings.diff_removed);
    let login = reports
        .iter()
        .find(|r| r.name == "login/linux-x86_64-default.png")
        .unwrap();
    assert!(login.diff_removed);

    // Untouched files survive.
    assert_eq!(
        fs::read(root.join("settings/macos-aarch64-metal.png")).unwrap(),
        b"baseline".as_slice()
    );
    assert_eq!(
        fs::read(root.join("settings/notes.txt")).unwrap(),
        b"keep me".as_slice()
    );

    // A second run finds nothing.
    let again = bless_dir(root, false).expect("idempotent");
    assert_eq!(again.len(), 0);
}

#[test]
fn dry_run_lists_without_touching() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = dir.path();
    write(root.join("login/linux-x86_64-default.png.new.png"), b"new");
    write(root.join("login/linux-x86_64-default.png"), b"old");

    let reports = bless_dir(root, true).expect("dry run");
    assert_eq!(reports.len(), 1);
    assert!(!reports[0].applied);

    // Nothing moved.
    assert_eq!(
        fs::read(root.join("login/linux-x86_64-default.png")).unwrap(),
        b"old".as_slice()
    );
    assert!(root.join("login/linux-x86_64-default.png.new.png").exists());
}

#[test]
fn missing_dir_is_a_clean_error() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let err = bless_dir(&dir.path().join("nope"), false).unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}
