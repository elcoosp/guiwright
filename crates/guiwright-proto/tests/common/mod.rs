//! Shared helpers for the proto golden test binaries.

/// Whether a `GUIWRIGHT_BLESS=1` run may write its golden file.
///
/// Under cargo-nextest (`NEXTEST` is set in every test process) the write
/// is refused: nextest fans each test out into its own fresh process, so a
/// bless env var leaked into the ambient environment would fan a fixed
/// repo-path write across all of them concurrently (last writer wins, torn
/// goldens). The caller falls through to the compare-against-committed
/// branch instead — bless deliberately via `cargo test`, which the doc
/// comment on each golden test prescribes.
pub fn bless_write_permitted(bless: Option<&str>, nextest: Option<&str>) -> bool {
    bless == Some("1") && nextest.is_none()
}

#[test]
fn bless_writes_only_outside_nextest() {
    // The documented flow: GUIWRIGHT_BLESS=1 under plain cargo test.
    assert!(bless_write_permitted(Some("1"), None));
    // Any nextest presence blocks the write (nextest sets NEXTEST=1 plus
    // a per-test uid; any value counts).
    assert!(!bless_write_permitted(Some("1"), Some("1")));
    assert!(!bless_write_permitted(Some("1"), Some("uid-7")));
    // Without the bless env the write never happens, nextest or not.
    assert!(!bless_write_permitted(None, None));
    assert!(!bless_write_permitted(None, Some("1")));
    // A bless value other than exactly "1" is not a bless request.
    assert!(!bless_write_permitted(Some("0"), None));
    assert!(!bless_write_permitted(Some(""), None));
}
