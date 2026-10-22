//! The lowering merges every imported package before anything else; a package
//! that cannot be read stops it, naming the file.

use crate::target::shared::lower::lower_project;
use crate::target::NativeBuildMode;

#[test]
fn an_imported_package_that_cannot_be_read_stops_the_lowering() {
    let ir = crate::testutil::named_ir_for_src(
        "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        "pkgprog",
    );
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("packages").join("ghost.mfp");
    let error = lower_project(
        &ir,
        "linux-x86_64".to_string(),
        std::slice::from_ref(&missing),
        NativeBuildMode::Console,
        None,
        crate::codegen::debug::DebugOptions::OFF,
    )
    .err()
    .expect("an unreadable package is refused");
    assert!(error.contains(&missing.display().to_string()), "{error}");
}
