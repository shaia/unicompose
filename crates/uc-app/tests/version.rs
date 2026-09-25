//! The release version lives in two places: the workspace manifest and the VERSION file.

#[test]
fn version_file_matches_the_workspace_version() {
    assert_eq!(include_str!("../../../VERSION").trim(), env!("CARGO_PKG_VERSION"));
}
