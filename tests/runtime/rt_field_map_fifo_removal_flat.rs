//! bug-768: removing the oldest key of a `Map` / `Set` held in a record or
//! `STATE` field costs the same at any size — as bug-760 B1 made it for a local.
//!
//! bug-760 B1 leaves a removed entry as a tombstone at a probed local or global
//! binding and compacts lazily, but a record/`STATE` field owner kept bug-732's
//! slide (`builder_inplace_assign/field.rs:lower_field_remove_key` /
//! `lower_field_set_remove` pass `tombstone = false`), because only a
//! `Local`/`Global` read settled a map. So a field-held FIFO step costs the
//! window: ~3 µs at W = 500, ~31 µs at W = 4000 (×10.4 for ×8), ~100× the local.
//!
//! **The measure** is `rt_map_fifo_removal_flat`'s: the same FIFO steps at `W` and
//! `8W`, in-process, best of three; flat per-step cost is near ×1, a per-removal
//! slide ×8; the bound is ×3. Every program also reads the survivors back through
//! the field, through a whole-record copy, and (for `STATE`) through `f.state`, so
//! a tombstone that leaks into any reader fails here too.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const W: u64 = 500;
const SCALE: u64 = 8;
const STEPS: u64 = 20_000;
const MAX_RATIO: f64 = 3.0;

const PROGRAM: &str = r#"IMPORT collections
IMPORT io
IMPORT datetime
IMPORT fs

TYPE RecM
  before AS Integer
  xs AS Map OF Integer TO Integer
  after AS Integer
END TYPE

TYPE RecS
  before AS Integer
  xs AS Set OF Integer
  after AS Integer
END TYPE

' The survivors must be keys STEPS .. STEPS + w - 1, in insertion order, with
' their values, and nothing removed may still answer.
FUNC checkMap(m AS Map OF Integer TO Integer, w AS Integer) AS Integer
  LET ks AS List OF Integer = collections::keys(m)
  MUT bad AS Integer = 0
  FOR j = 0 TO len(ks) - 1
    LET k AS Integer = collections::get(ks, j)
    IF k <> STEPS + j OR collections::get(m, k) <> k * 3 THEN
      bad = bad + 1
    END IF
  NEXT
  IF len(m) <> w OR len(ks) <> w OR collections::hasKey(m, STEPS - 1) THEN
    bad = bad + 1
  END IF
  RETURN bad
END FUNC

FUNC checkSet(s AS Set OF Integer, w AS Integer) AS Integer
  LET ks AS List OF Integer = collections::toList(s)
  MUT bad AS Integer = 0
  FOR j = 0 TO len(ks) - 1
    IF collections::get(ks, j) <> STEPS + j THEN
      bad = bad + 1
    END IF
  NEXT
  IF len(s) <> w OR len(ks) <> w OR collections::contains(s, STEPS - 1) THEN
    bad = bad + 1
  END IF
  RETURN bad
END FUNC

FUNC recMap(w AS Integer) AS Integer
  MUT rec AS RecM = RecM[before := 1, xs := Map OF Integer TO Integer { }, after := 2]
  FOR i = 0 TO w - 1
    rec = WITH rec { xs := collections::set(rec.xs, i, i * 3) }
  NEXT
  FOR i = w TO w + STEPS - 1
    rec = WITH rec { xs := collections::set(rec.xs, i, i * 3) }
    rec = WITH rec { xs := collections::removeKey(rec.xs, i - w) }
  NEXT
  LET copy AS RecM = rec
  RETURN checkMap(rec.xs, w) + checkMap(copy.xs, w) + rec.before + rec.after - 3
END FUNC

FUNC recSet(w AS Integer) AS Integer
  MUT rec AS RecS = RecS[before := 1, xs := Set OF Integer { }, after := 2]
  FOR i = 0 TO w - 1
    rec = WITH rec { xs := collections::add(rec.xs, i) }
  NEXT
  FOR i = w TO w + STEPS - 1
    rec = WITH rec { xs := collections::add(rec.xs, i) }
    rec = WITH rec { xs := collections::remove(rec.xs, i - w) }
  NEXT
  LET copy AS RecS = rec
  RETURN checkSet(rec.xs, w) + checkSet(copy.xs, w) + rec.before + rec.after - 3
END FUNC

FUNC stateMap(w AS Integer) AS Integer
  LET path AS String = fs::pathJoin([fs::tempDirectory(), "b768_state_" & toString(w) & ".tmp"])
  RES f AS fs::File STATE RecM = fs::open(path, "write")
  fs::deleteFile(path)
  f.state.xs = Map OF Integer TO Integer { }
  FOR i = 0 TO w - 1
    f.state.xs = collections::set(f.state.xs, i, i * 3)
  NEXT
  FOR i = w TO w + STEPS - 1
    f.state.xs = collections::set(f.state.xs, i, i * 3)
    f.state.xs = collections::removeKey(f.state.xs, i - w)
  NEXT
  RETURN checkMap(f.state.xs, w)
END FUNC

FUNC best(which AS String, w AS Integer) AS Integer
  MUT bestNs AS Integer = 9223372036854775807
  FOR r = 0 TO 2
    LET t0 AS Integer = datetime::monotonicNanos()
    MUT bad AS Integer = 0
    IF which = "record-map" THEN
      bad = recMap(w)
    ELSEIF which = "record-set" THEN
      bad = recSet(w)
    ELSE
      bad = stateMap(w)
    END IF
    LET ns AS Integer = datetime::monotonicNanos() - t0
    IF bad <> 0 THEN
      io::print("BAD " & which & " " & toString(w) & " " & toString(bad))
    END IF
    IF ns < bestNs THEN
      bestNs = ns
    END IF
  NEXT
  RETURN bestNs
END FUNC

FUNC main AS Integer
  io::print("record-map " & toString(best("record-map", W)) & " " & toString(best("record-map", W * SCALE)))
  io::print("record-set " & toString(best("record-set", W)) & " " & toString(best("record-set", W * SCALE)))
  io::print("state-map " & toString(best("state-map", W)) & " " & toString(best("state-map", W * SCALE)))
  RETURN 0
END FUNC
"#;

fn program() -> String {
    PROGRAM
        .replace("SCALE", &SCALE.to_string())
        .replace("STEPS", &STEPS.to_string())
        .replace(" W)", &format!(" {W})"))
        .replace(" W *", &format!(" {W} *"))
}

#[test]
fn removing_the_oldest_key_of_a_field_is_flat_in_the_window_size() {
    let project = common::temp_project("b768_field_fifo_removal", &program());
    let exe = common::build_project(&project);
    let output = Command::new(&exe).output().expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "program failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !stdout.contains("BAD"),
        "bug-768: field-held FIFO churn lost, reordered or exposed entries:\n{stdout}"
    );
    let mut failures = Vec::new();
    for line in stdout.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [kind, small, large] = fields[..] else {
            panic!("unexpected line {line:?} in:\n{stdout}");
        };
        let small: f64 = small.parse().expect("small ns");
        let large: f64 = large.parse().expect("large ns");
        let ratio = large / small;
        eprintln!(
            "{kind}: W={W}: {:.2} ms; W={}: {:.2} ms; ×{ratio:.1}",
            small / 1e6,
            W * SCALE,
            large / 1e6
        );
        if ratio > MAX_RATIO {
            failures.push(format!(
                "{kind}: {STEPS} FIFO steps took {:.2} ms at a window of {W} and {:.2} ms at \
                 {}: ×{ratio:.1}",
                small / 1e6,
                large / 1e6,
                W * SCALE
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "bug-768: removing the oldest key of a field costs the window size (flat is ≤ \
         ×{MAX_RATIO}; a per-removal slide is ×{SCALE}):\n  {}",
        failures.join("\n  ")
    );
    let _ = std::fs::remove_dir_all(&project);
}
