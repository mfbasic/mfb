//! `mfb build` is deterministic: the same source built repeatedly in the same
//! process environment produces byte-identical native code (plan-118-D).
//!
//! The whole byte-identity gate — 1,823 goldens, `artifact-gate.sh`,
//! `.ncodesum` — silently assumes this. Nothing checked it, and plan-118-D
//! broke it: the per-type constructor synthesis iterated a `HashMap` to decide
//! the ORDER `construct.T` functions are emitted in, and function order is
//! observable in the `.ncode`. Three builds of one fixture produced three
//! different `sha256`s. The failure mode is nasty because it does not look like
//! a compiler bug from the gate's side — it looks like a flaky golden, and the
//! repair one reaches for is to regenerate the golden, which "fixes" it until
//! the next run.
//!
//! The source below is chosen to exercise the compiler's collection-ordered
//! emission seams: several record types constructed enough times to qualify for
//! synthesis, a `String` field (so the record has an inlined data region), and
//! errors (so the error-plumbing types are constructed too).

mod common;
use common::temp_project;
use std::process::Command;

const SOURCE: &str = r#"
IMPORT io

TYPE Alpha
  name AS String
  n AS Integer
END TYPE

TYPE Beta
  n AS Integer
  m AS Integer
END TYPE

TYPE Gamma
  label AS String
END TYPE

FUNC checked(n AS Integer) AS Integer
  IF n < 0 THEN FAIL error(100, "negative")
  RETURN n
END FUNC

FUNC main AS Integer
  LET a1 AS Alpha = Alpha["one", 1]
  LET a2 AS Alpha = Alpha["two", 2]
  LET a3 AS Alpha = Alpha["three", 3]
  LET b1 AS Beta = Beta[1, 2]
  LET b2 AS Beta = Beta[3, 4]
  LET b3 AS Beta = Beta[5, 6]
  LET g1 AS Gamma = Gamma["x"]
  LET g2 AS Gamma = Gamma["y"]
  LET g3 AS Gamma = Gamma["z"]
  MUT total AS Integer = a1.n + a2.n + a3.n + b1.n + b2.m + b3.n
  LET one AS Integer = checked(1) TRAP(e)
    RECOVER 0
  END TRAP
  LET two AS Integer = checked(2) TRAP(e)
    RECOVER 0
  END TRAP
  total = total + one + two
  io::print(a1.name & g1.label & g2.label & g3.label & toString(total))
  RETURN 0
END FUNC
"#;

/// Build the same project three times, hashing the native code plan each time.
///
/// The `-ncode` dump rather than the linked executable: it is the deterministic
/// artifact the gate actually compares, and it names every emitted function in
/// emission order, which is precisely what a hash-ordered synthesis perturbs.
#[test]
fn native_code_plan_is_byte_identical_across_repeated_builds() {
    let project = temp_project("build_determinism", SOURCE);
    let mut dumps = Vec::new();
    for attempt in 0..3 {
        let output = Command::new(common::mfb_exe())
            .arg("build")
            .arg("-q")
            .arg("-ncode")
            .arg(&project)
            .output()
            .expect("run mfb build -ncode");
        assert!(
            output.status.success(),
            "build {attempt} failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let dump = project.join("build_determinism.ncode");
        dumps.push(std::fs::read(&dump).unwrap_or_else(|err| {
            panic!("read {}: {err}", dump.display());
        }));
    }
    for (attempt, dump) in dumps.iter().enumerate().skip(1) {
        assert_eq!(
            dump.len(),
            dumps[0].len(),
            "build {attempt} produced a native code plan of a different length \
             ({} vs {} bytes) — codegen is not deterministic",
            dump.len(),
            dumps[0].len()
        );
        if dump != &dumps[0] {
            // Report the first divergence rather than dumping megabytes.
            let first = dump
                .iter()
                .zip(&dumps[0])
                .position(|(left, right)| left != right)
                .expect("lengths matched but contents differ");
            let window = first.saturating_sub(80)..(first + 80).min(dump.len());
            panic!(
                "build {attempt} diverges from build 0 at byte {first} — codegen is \
                 not deterministic.\nbuild 0: {:?}\nbuild {attempt}: {:?}",
                String::from_utf8_lossy(&dumps[0][window.clone()]),
                String::from_utf8_lossy(&dump[window])
            );
        }
    }
}
