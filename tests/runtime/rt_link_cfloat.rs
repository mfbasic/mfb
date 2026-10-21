//! bug-788: a `LINK` `CFloat` crossed the boundary as raw binary64 bits in an
//! integer register. A C `float` parameter read garbage, a `float` return was
//! taken from the integer return register, and an OUT `CFloat` (like an OUT
//! `CDouble`) had its *pointer* staged into an FP register, so the callee wrote
//! through garbage (SIGSEGV). Win64 also assigned argument registers per bank
//! rather than by position.
//!
//! The oracle is a real C library compiled here. Every row is a value only a
//! faithful binary32 path produces. `0.1f / 2` and `0.1f` widen to their exact
//! binary32 values, and `mixed` checks each argument's position. Narrowing a
//! value too large for binary32 raises `ErrFloatOverflow` (77050015), and a
//! non-finite `float` return raises the same `ErrFloatInf` (77050014) a `CDouble`
//! return does.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::process::Command;

const PROBE_C: &str = r#"#include <stdint.h>
typedef struct { int32_t tag; float f; double d; } Pair;
int32_t check_float(float x) { return x == 1.5f; }
float half_float(float x) { return x / 2.0f; }
double mixed(int32_t a, float b, int32_t c, double d) { return a + (double)b * 10 + c * 100 + d * 1000; }
int32_t out_float(float x, float *y) { *y = x * 3.0f; return 0; }
int32_t pair_in(Pair *p) { return p->tag == 7 && p->f == 2.5f && p->d == 4.0; }
int32_t pair_out(Pair *p) { p->tag = 9; p->f = 0.1f; p->d = 0.25; return 0; }
float make_inf(void) { volatile float big = 1e30f; return big * big; }
int32_t out_double(double x, double *y) { *y = x * 2.0; return 0; }
"#;

const MAIN_MFB: &str = r#"IMPORT io

TYPE Duo
  tag AS Integer
  f AS Float
  d AS Float
END TYPE

LINK "probe" AS probe
  CSTRUCT CDuo AS Duo
    tag CInt32
    f CFloat
    d CDouble
  END CSTRUCT

  FUNC checkFloat(x AS Float) AS Integer
    SYMBOL "check_float"
    ABI (x CFloat) AS value CInt32
    RETURN value
  END FUNC

  FUNC halfFloat(x AS Float) AS Float
    SYMBOL "half_float"
    ABI (x CFloat) AS value CFloat
    RETURN value
  END FUNC

  FUNC mixed(a AS Integer, b AS Float, c AS Integer, d AS Float) AS Float
    SYMBOL "mixed"
    ABI (a CInt32, b CFloat, c CInt32, d CDouble) AS value CDouble
    RETURN value
  END FUNC

  FUNC tripled(x AS Float) AS Float
    SYMBOL "out_float"
    ABI (x CFloat, y OUT CFloat) AS status CInt32
    RETURN y
    SUCCESS_ON status = 0
  END FUNC

  FUNC pairIn(fv AS Float) AS Integer
    SYMBOL "pair_in"
    ABI (p IN CDuo) AS value CInt32
    BIND IN p
      tag = 7
      f = fv
      d = 4
    END BIND
    RETURN value
  END FUNC

  FUNC pairOut() AS Duo
    SYMBOL "pair_out"
    ABI (p OUT CDuo) AS status CInt32
    RETURN p
    SUCCESS_ON status = 0
  END FUNC

  FUNC doubled(x AS Float) AS Float
    SYMBOL "out_double"
    ABI (x CDouble, y OUT CDouble) AS status CInt32
    RETURN y
    SUCCESS_ON status = 0
  END FUNC

  FUNC makeInf() AS Float
    SYMBOL "make_inf"
    ABI () AS value CFloat
    RETURN value
  END FUNC
END LINK

FUNC main() AS Nothing
  io::print("check " & toString(probe::checkFloat(1.5)))
  io::print("half " & toString(probe::halfFloat(3.0)))
  IF probe::halfFloat(0.1) = 0.05 THEN
    io::print("half0.1 double")
  ELSE
    IF probe::halfFloat(0.1) = 0.05000000074505806 THEN
      io::print("half0.1 binary32")
    ELSE
      io::print("half0.1 other")
    END IF
  END IF
  io::print("mixed " & toString(probe::mixed(1, 2.5, 3, 4.0)))
  io::print("outd " & toString(probe::doubled(1.25)))
  io::print("out " & toString(probe::tripled(1.5)))
  io::print("pairIn " & toString(probe::pairIn(2.5)))
  LET p = probe::pairOut()
  io::print("pairOut " & toString(p.tag) & " " & toString(p.d))
  IF p.f = 0.10000000149011612 THEN
    io::print("pairOut f binary32")
  ELSE
    io::print("pairOut f other")
  END IF
  LET big = probe::checkFloat(1.0e300) TRAP(e)
    io::print("narrow " & toString(e.code))
    RECOVER -1
  END TRAP
  LET inf = probe::makeInf() TRAP(e)
    io::print("inf " & toString(e.code))
    RECOVER 0.0
  END TRAP
END FUNC
"#;

#[test]
fn cfloat_crosses_every_link_boundary_as_binary32() {
    let root = std::env::temp_dir().join(format!("mfb_bug788_{}", common::unique_nonce()));
    fs::create_dir_all(root.join("src")).expect("create project");
    fs::create_dir_all(root.join("vendor")).expect("create vendor dir");
    let source = root.join("probe.c");
    fs::write(&source, PROBE_C).expect("write probe.c");
    // A Linux build emits both libc worlds of the host arch in one pass and
    // resolves the library for each, so each needs a locator, and `vendor/` is flat,
    // so each locator needs its own filename. Only the host's executable runs, so
    // the one library the host `cc` builds is copied under both names.
    let (libraries, locator): (Vec<String>, String) = if cfg!(target_os = "macos") {
        (
            vec!["libprobe.dylib".to_string()],
            r#"{"os":"macos","type":"vendor","source":"libprobe.dylib"}"#.to_string(),
        )
    } else {
        let flavors = ["glibc", "musl"];
        let locators: Vec<String> = flavors
            .iter()
            .map(|libc| {
                format!(
                    r#"{{"os":"linux","arch":"{}","libc":"{libc}","type":"vendor","source":"libprobe-{libc}.so"}}"#,
                    std::env::consts::ARCH,
                )
            })
            .collect();
        (
            flavors
                .iter()
                .map(|libc| format!("libprobe-{libc}.so"))
                .collect(),
            locators.join(","),
        )
    };
    let mut cc = Command::new("cc");
    if cfg!(target_os = "macos") {
        cc.arg("-dynamiclib");
    } else {
        cc.args(["-shared", "-fPIC"]);
    }
    let built = root.join("vendor").join(&libraries[0]);
    let compiled = cc
        .arg("-o")
        .arg(&built)
        .arg(&source)
        .output()
        .expect("run cc");
    assert!(
        compiled.status.success(),
        "probe library build failed: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    for copy in &libraries[1..] {
        fs::copy(&built, root.join("vendor").join(copy)).expect("copy the probe library");
    }
    fs::write(
        root.join("project.json"),
        format!(
            r#"{{"name":"cfloat","version":"0.1.0","mfb":"1.0","kind":"executable","entry":"main","sources":[{{"root":"src","role":"main","include":["**/*.mfb"]}}],"targets":["native"],"libraries":{{"probe":[{locator}]}}}}"#
        ),
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), MAIN_MFB).expect("write main.mfb");

    let executable = common::build_project(&root);
    let output = common::run_with_stdin(&executable, b"");
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "check 1",
            "half 1.50",
            "half0.1 binary32",
            "mixed 4326.00",
            "outd 2.50",
            "out 4.50",
            "pairIn 1",
            "pairOut 9 0.25",
            "pairOut f binary32",
            "narrow 77050015",
            "inf 77050014",
        ]
    );
}
