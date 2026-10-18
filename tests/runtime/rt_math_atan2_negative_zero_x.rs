//! bug-932: `math::atan2(y, x)` with `x` negative zero is `+pi/2` for `y > 0` and
//! `-pi/2` for `y < 0` (IEEE 754 / C Annex F), on the scalar and the
//! `List OF Float` overloads alike. The quadrant correction used an ordered
//! `x < 0` compare, for which `-0.0 < 0` is false, so both signs came out
//! negated. The origin stays 0 (bug-131) and the other quadrants are unchanged.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

#[test]
fn atan2_with_negative_zero_x_has_the_sign_of_y() {
    let source = "IMPORT io
IMPORT math
IMPORT os

FUNC main() AS Nothing
  LET z AS Float = toFloat(os::getEnvOr(\"MFB_UNSET_ATAN2_ZERO\", \"0.0\"))
  LET nz AS Float = -1.0 * z
  LET one AS Float = toFloat(os::getEnvOr(\"MFB_UNSET_ATAN2_ONE\", \"1.0\"))
  io::print(\"s1 \" & toString(math::atan2(one, nz), toByte(6)))
  io::print(\"s2 \" & toString(math::atan2(-one, nz), toByte(6)))
  io::print(\"s3 \" & toString(math::atan2(one, z), toByte(6)))
  io::print(\"s4 \" & toString(math::atan2(-one, z), toByte(6)))
  io::print(\"s5 \" & toString(math::atan2(z, nz), toByte(6)))
  io::print(\"s6 \" & toString(math::atan2(nz, nz), toByte(6)))
  io::print(\"s7 \" & toString(math::atan2(z, -one), toByte(6)))
  io::print(\"s8 \" & toString(math::atan2(nz, -one), toByte(6)))
  io::print(\"s9 \" & toString(math::atan2(-one, -one), toByte(6)))
  io::print(\"s10 \" & toString(math::atan2(one, one), toByte(6)))
  FOR EACH v IN math::atan2([one, -one, 2.0, 0.0, 0.0, -one], [nz, nz, nz, nz, -one, -one])
    io::print(\"list \" & toString(v, toByte(6)))
  NEXT
END FUNC
";
    let project = common::temp_project("atan2_negative_zero_x", source);
    let exe = common::build_project(&project);
    let out = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    assert!(out.status.success(), "program failed");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "s1 1.570796
s2 -1.570796
s3 1.570796
s4 -1.570796
s5 0.000000
s6 0.000000
s7 3.141593
s8 -3.141593
s9 -2.356194
s10 0.785398
list 1.570796
list -1.570796
list 1.570796
list 0.000000
list 3.141593
list -2.356194
"
    );
}
