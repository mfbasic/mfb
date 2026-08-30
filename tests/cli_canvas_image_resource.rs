//! `canvas::Image` behaves exactly like every other RES resource (plan-98-B Phase 4).
//!
//! An `Image` is a plain scope-owned resource on the canonical record header —
//! `tag@0`, `handle@8`, `closed@16`, `STATE@24` — with no refcount and no generation
//! table. What that buys, and what these check, is that a *scene* can name an image
//! without owning it: the scene carries an `ImageRef` (an integer id), so destroying
//! an image a presented scene still draws is safe rather than a dangling reference.
//!
//! The exit codes are distinct per property so a regression names what broke rather
//! than reporting "non-zero".
//!
//! One thing deliberately NOT tested from source: closing twice in a row, and
//! reading after an in-scope close. The compiler rejects both statically
//! (`TYPE_USE_AFTER_MOVE`), which is a stronger guarantee than the runtime no-op —
//! so the `ErrResourceClosed` case is reached the way it actually occurs, by closing
//! through a `RES` parameter, where ownership floats up and the caller's binding is
//! closed without the checker being able to see it.

mod common;
use common::temp_project;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// A resource type is referenced **package-qualified** (`canvas::Image`), like
/// `fs::File` — unlike the value types (`Color`, `DrawItem`), which are bare.
const SOURCE: &str = r#"IMPORT app
IMPORT canvas
IMPORT collections
IMPORT errorCode
IMPORT io

' createImage rejects a pixel list whose length is not width*height*4.
FUNC badCreate() AS Integer
  LET short AS List OF Byte = [toByte(1), toByte(2)]
  RES bad AS canvas::Image = canvas::createImage(4, 4, short) TRAP(err)
    IF err.code = errorCode::ErrBadPixelCount THEN
      RETURN 0
    END IF
    RETURN 20
  END TRAP
  RETURN 21
END FUNC

' Closing through a RES parameter: ownership floats up, so the caller's binding is
' the same resource and is closed when this returns. The compiler cannot see that,
' which is exactly the path the runtime closed-flag guard exists for.
SUB closeIt(RES img AS canvas::Image)
  canvas::destroyImage(img)
END SUB

FUNC closedRefuses() AS Integer
  LET px AS List OF Byte = [toByte(1), toByte(2), toByte(3), toByte(4)]
  RES img AS canvas::Image = canvas::createImage(1, 1, px)
  closeIt(img)
  LET dead AS Size = canvas::getSize(img) TRAP(err)
    IF err.code = errorCode::ErrResourceClosed THEN
      RETURN 0
    END IF
    RETURN 30
  END TRAP
  RETURN 31
END FUNC

FUNC setBytesRejectsWrongLength() AS Integer
  LET px AS List OF Byte = [toByte(10), toByte(20), toByte(30), toByte(40), toByte(50), toByte(60), toByte(70), toByte(80)]
  RES img AS canvas::Image = canvas::createImage(2, 1, px)
  LET short AS List OF Byte = [toByte(1), toByte(2)]
  canvas::setBytes(img, short) TRAP(err)
    IF err.code = errorCode::ErrBadPixelCount THEN
      RETURN 0
    END IF
    RETURN 40
  END TRAP
  RETURN 41
END FUNC

FUNC main AS Integer
  app::setMode(Mode.Canvas)

  LET px AS List OF Byte = [toByte(10), toByte(20), toByte(30), toByte(40), toByte(50), toByte(60), toByte(70), toByte(80)]
  RES img AS canvas::Image = canvas::createImage(2, 1, px)

  LET size AS Size = canvas::getSize(img)
  IF size.width <> 2 THEN
    RETURN 1
  END IF
  IF size.height <> 1 THEN
    RETURN 2
  END IF

  LET back AS List OF Byte = canvas::getBytes(img)
  IF len(back) <> 8 THEN
    RETURN 3
  END IF
  IF collections::getOr(back, 0, toByte(0)) <> toByte(10) THEN
    RETURN 4
  END IF
  IF collections::getOr(back, 7, toByte(0)) <> toByte(80) THEN
    RETURN 5
  END IF

  LET handle AS ImageRef = canvas::imageRef(img)
  IF handle.id = 0 THEN
    RETURN 6
  END IF

  LET fresh AS List OF Byte = [toByte(90), toByte(91), toByte(92), toByte(93), toByte(94), toByte(95), toByte(96), toByte(97)]
  canvas::setBytes(img, fresh)
  LET after AS List OF Byte = canvas::getBytes(img)
  IF collections::getOr(after, 0, toByte(0)) <> toByte(90) THEN
    RETURN 7
  END IF
  IF collections::getOr(after, 7, toByte(0)) <> toByte(97) THEN
    RETURN 8
  END IF

  ' The scene carries the handle, not the image.
  LET tile AS DrawItem = Picture[x := 0.0, y := 0.0, w := 4.0, h := 2.0, image := handle, paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::present([tile])

  LET r1 AS Integer = badCreate()
  IF r1 <> 0 THEN
    RETURN r1
  END IF
  LET r2 AS Integer = closedRefuses()
  IF r2 <> 0 THEN
    RETURN r2
  END IF
  LET r3 AS Integer = setBytesRejectsWrongLength()
  IF r3 <> 0 THEN
    RETURN r3
  END IF

  io::print("IMAGE_OK")
  RETURN 0
END FUNC
"#;

fn build(name: &str, extra: &[&str]) -> (PathBuf, bool, String) {
    let project = temp_project(name, SOURCE);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-app")
        .args(extra)
        .arg(&project)
        .output()
        .expect("run mfb build -app");
    let combined = format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (project, output.status.success(), combined)
}

#[test]
fn the_image_surface_compiles_for_the_host() {
    let (project, ok, log) = build("canvas_image_build", &[]);
    assert!(ok, "the canvas image surface should compile:\n{log}");
    let _ = fs::remove_dir_all(&project);
}

/// Every `--app` backend must advertise the image calls, or a program using them is
/// rejected at `validate_capabilities` on that target.
#[test]
fn the_image_surface_compiles_for_the_other_app_targets() {
    for target in ["linux-aarch64", "linux-x86_64", "windows-x86_64"] {
        let (project, ok, log) = build("canvas_image_cross", &["-target", target]);
        assert!(
            ok,
            "{target} should accept the canvas image surface:\n{log}"
        );
        let _ = fs::remove_dir_all(&project);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_the_image_resource_contract_holds_at_runtime() {
    let (project, ok, log) = build("canvas_image_rt", &[]);
    assert!(ok, "build should succeed:\n{log}");
    let exe = project.join("build/canvas_image_rt.app/Contents/MacOS/canvas_image_rt");
    let output = Command::new(&exe)
        .env("MFB_MACAPP_HEADLESS", "1")
        .output()
        .expect("run headless app bundle");
    let code = output.status.code().unwrap_or(-1);
    assert_eq!(
        code, 0,
        "image resource contract failed with code {code} \
         (1-2 getSize, 3-5 getBytes, 6 imageRef, 7-8 setBytes round-trip, \
         20-21 createImage pixel count, 30-31 closed guard, 40-41 setBytes count)"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "IMAGE_OK\n",
        "the program must run to completion"
    );
    let _ = fs::remove_dir_all(&project);
}
