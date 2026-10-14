//! bug-784: updating a record nested inside a list element — `LET h = get(hs, k)`
//! then `hs = set(hs, k, WITH h { inner := …, … })` — writes the new `inner` over
//! the element's own bytes when its size is unchanged, instead of rebuilding the
//! whole element in a fresh block per update (M2), and an inline
//! `inner := WITH h.inner { <scalars> }` stores those scalars into the element's
//! `inner` sub-block without building a `Wide` at all (M1, inline form), and
//! `inner := stepWide(h.inner, …)` calls `stepWide`'s destination variant, which
//! writes its result straight into `h.inner`'s bytes (M1, call form).
//!
//! Each case runs `REPS` and `2 × REPS` rounds of 64 updates under `--debug`. The
//! allocation growth per update must stay within the case's budget, nothing may
//! leak, and the case must print what its **contrast** prints: the same updates
//! through a plain owned copy (`ident(get(…))`), which the element-bound analysis
//! does not see and which takes the general copying `WITH` and `set`.
//!
//! Budgets: an update whose new `inner` is the old one's size allocates nothing,
//! through a helper call (`stepWide`) too. The `overlap_*` cases pin the
//! destination variant's aliasing: the callee reads the destination it writes
//! (its own fields, an argument that is a view into it), and a later update that
//! reads the destination must not see the new value — that one keeps the call's
//! result block, 1 per update.
//! The `*_resize` cases change `inner`'s size on every update, so they take the
//! rebuild (the `WITH` block, plus the call's) and are here for their output: the
//! size-change branch must store the values the in-place branch computed, once.
//! Before the fix every case but `nestedtouch` allocated 2 per update.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const REPS: u32 = 40;

const PRELUDE: &str = "\
IMPORT io
IMPORT collections
IMPORT math

TYPE Wide
  id AS Integer
  count AS Integer
  total AS Integer
  flags AS Integer
  pos AS Float
  vel AS Float
  energy AS Float
  name AS String
  tag AS String
  weight AS Float
END TYPE

TYPE Holder
  id AS Integer
  label AS String
  inner AS Wide
  touches AS Integer
END TYPE

TYPE FHolder
  label AS String
  inner AS Wide
  score AS Float
END TYPE

TYPE LateHolder
  inner AS Wide
  label AS String
  touches AS Integer
END TYPE

FUNC stepWide(w AS Wide, i AS Integer, tag AS String) AS Wide
  RETURN WITH w { count := w.count + 1, total := w.total + i MOD 7, flags := (w.flags * 5 + i) MOD 1024, pos := w.pos + w.vel * 0.25, vel := w.vel * 0.5 + toFloat(i MOD 4), energy := w.energy + w.vel, tag := tag }
END FUNC

FUNC stepSelf(w AS Wide, i AS Integer) AS Wide
  RETURN WITH w { count := w.count + 1, total := w.count + w.total + i MOD 3, tag := w.name, pos := w.pos + w.vel }
END FUNC

FUNC newWide(id AS Integer) AS Wide
  RETURN Wide[id := id, count := 0, total := id, flags := id MOD 1024, pos := toFloat(id) * 0.5, vel := 1.0, energy := 0.0, name := \"wide-\" & toString(id), tag := \"\", weight := toFloat(id) * 0.125]
END FUNC

FUNC ident(h AS Holder) AS Holder
  RETURN h
END FUNC

FUNC identF(h AS FHolder) AS FHolder
  RETURN h
END FUNC

FUNC identL(h AS LateHolder) AS LateHolder
  RETURN h
END FUNC

FUNC wideSum(w AS Wide) AS Integer
  RETURN w.id + w.count + w.total + w.flags + math::floor(w.pos * 4.0) + math::floor(w.vel * 1024.0) + math::floor(w.energy) + len(w.name) * 3 + len(w.tag) * 5 + math::floor(w.weight * 8.0)
END FUNC

FUNC holders() AS List OF Holder
  MUT hs AS List OF Holder = []
  FOR k = 0 TO 63
    hs = collections::append(hs, Holder[id := k, label := \"holder-\" & toString(k), inner := newWide(k), touches := 0])
  NEXT
  RETURN hs
END FUNC

FUNC fholders() AS List OF FHolder
  MUT hs AS List OF FHolder = []
  FOR k = 0 TO 63
    hs = collections::append(hs, FHolder[label := \"f-\" & toString(k), inner := newWide(k), score := 0.25])
  NEXT
  RETURN hs
END FUNC

FUNC lateholders() AS List OF LateHolder
  MUT hs AS List OF LateHolder = []
  FOR k = 0 TO 63
    hs = collections::append(hs, LateHolder[inner := newWide(k), label := \"late-\" & toString(k), touches := 0])
  NEXT
  RETURN hs
END FUNC

FUNC sumHolders(hs AS List OF Holder) AS Integer
  MUT acc AS Integer = 0
  FOR EACH h IN hs
    acc = (acc * 31 + h.id + h.touches * 7 + len(h.label) + wideSum(h.inner)) MOD 1000000007
  NEXT
  RETURN acc
END FUNC

FUNC sumF(hs AS List OF FHolder) AS Integer
  MUT acc AS Integer = 0
  FOR EACH h IN hs
    acc = (acc * 31 + math::floor(h.score * 4.0) + len(h.label) + wideSum(h.inner)) MOD 1000000007
  NEXT
  RETURN acc
END FUNC

FUNC sumL(hs AS List OF LateHolder) AS Integer
  MUT acc AS Integer = 0
  FOR EACH h IN hs
    acc = (acc * 31 + h.touches * 7 + len(h.label) + wideSum(h.inner)) MOD 1000000007
  NEXT
  RETURN acc
END FUNC
";

/// `(name, setup, body, contrast body, result, allocations allowed per update)`.
/// `body` runs once per update with `i` the update number and `k` its element.
/// `None`: an output-only case (its size changes every update, so the relocating
/// `set` also grows the list's data region now and then).
const CASES: &[(&str, &str, &str, &str, &str, Option<u64>)] = &[
    (
        // The benchmark row: each element always sees the same tag (k and i share
        // parity), so after its first update `inner` keeps its size.
        "nested",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      IF i MOD 2 = 0 THEN\n        hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagEven), touches := h.touches + 1 })\n      ELSE\n        hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })\n      END IF",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      IF i MOD 2 = 0 THEN\n        hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagEven), touches := h.touches + 1 })\n      ELSE\n        hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })\n      END IF",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        "nestedsame",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        "nestedmut",
        "  MUT hs AS List OF Holder = holders()",
        "      MUT h AS Holder = collections::get(hs, k)\n      h = WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 }\n      hs = collections::set(hs, k, h)",
        "      MUT h AS Holder = ident(collections::get(hs, k))\n      h = WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 }\n      hs = collections::set(hs, k, h)",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        "nestedinner",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := WITH h.inner { count := h.inner.count + 1, pos := h.inner.pos + h.inner.vel }, touches := h.touches + 1 })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := WITH h.inner { count := h.inner.count + 1, pos := h.inner.pos + h.inner.vel }, touches := h.touches + 1 })",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        "nestedinner_mut",
        "  MUT hs AS List OF Holder = holders()",
        "      MUT h AS Holder = collections::get(hs, k)\n      h = WITH h { touches := h.touches + h.inner.count, inner := WITH h.inner { count := h.inner.count + 1, total := h.touches } }\n      hs = collections::set(hs, k, h)",
        "      MUT h AS Holder = ident(collections::get(hs, k))\n      h = WITH h { touches := h.touches + h.inner.count, inner := WITH h.inner { count := h.inner.count + 1, total := h.touches } }\n      hs = collections::set(hs, k, h)",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        "nestedtouch",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { touches := h.touches + 1 })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { touches := h.touches + 1 })",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        // The same element alternates between two tag lengths: every update
        // changes `inner`'s size and takes the size-change branch.
        "nested_resize",
        "  MUT hs AS List OF Holder = holders()",
        "      LET kk AS Integer = (i / 2) MOD 64\n      LET h AS Holder = collections::get(hs, kk)\n      IF i MOD 2 = 0 THEN\n        hs = collections::set(hs, kk, WITH h { inner := stepWide(h.inner, i, tagEven), touches := h.touches + 1 })\n      ELSE\n        hs = collections::set(hs, kk, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })\n      END IF",
        "      LET kk AS Integer = (i / 2) MOD 64\n      LET h AS Holder = ident(collections::get(hs, kk))\n      IF i MOD 2 = 0 THEN\n        hs = collections::set(hs, kk, WITH h { inner := stepWide(h.inner, i, tagEven), touches := h.touches + 1 })\n      ELSE\n        hs = collections::set(hs, kk, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })\n      END IF",
        "sumHolders(hs)",
        None,
    ),
    (
        // A `Float` scalar beside the replaced field, both branches.
        "float_scalar_resize",
        "  MUT hs AS List OF FHolder = fholders()",
        "      LET kk AS Integer = (i / 3) MOD 64\n      LET h AS FHolder = collections::get(hs, kk)\n      MUT t AS String = tagOdd\n      IF i MOD 3 = 0 THEN\n        t = tagEven\n      END IF\n      hs = collections::set(hs, kk, WITH h { score := h.score * 0.5 + toFloat(i MOD 5), inner := stepWide(h.inner, i, t) })",
        "      LET kk AS Integer = (i / 3) MOD 64\n      LET h AS FHolder = identF(collections::get(hs, kk))\n      MUT t AS String = tagOdd\n      IF i MOD 3 = 0 THEN\n        t = tagEven\n      END IF\n      hs = collections::set(hs, kk, WITH h { score := h.score * 0.5 + toFloat(i MOD 5), inner := stepWide(h.inner, i, t) })",
        "sumF(hs)",
        None,
    ),
    (
        // `inner` is not the last inlined field: a same-size `inner` still leaves
        // `label`'s offset where it was, so it is written in place too.
        "not_last_inlined",
        "  MUT hs AS List OF LateHolder = lateholders()",
        "      LET h AS LateHolder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })",
        "      LET h AS LateHolder = identL(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })",
        "sumL(hs)",
        Some(0),
    ),
    (
        // The callee reads the destination it writes: `total` reads the OLD
        // `count`, `tag` copies the destination's own `name` span.
        "overlap_callee_reads_destination",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := stepSelf(h.inner, i), touches := h.touches + 1 })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := stepSelf(h.inner, i), touches := h.touches + 1 })",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        // The `tag` argument is a view into the destination itself (its `name`).
        "overlap_argument_views_destination",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, h.inner.name), touches := h.touches + 1 })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, h.inner.name), touches := h.touches + 1 })",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        // The `tag` argument is the very span the callee replaces.
        "overlap_argument_is_replaced_span",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, h.inner.tag & \"x\"), touches := h.touches + 1 })\n      LET g AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH g { inner := stepWide(g.inner, i, g.inner.tag), touches := g.touches + 1 })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, h.inner.tag & \"x\"), touches := h.touches + 1 })\n      LET g AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH g { inner := stepWide(g.inner, i, g.inner.tag), touches := g.touches + 1 })",
        "sumHolders(hs)",
        None,
    ),
    (
        // A later update reads the destination: it must see the OLD `inner`, so
        // the destination variant is not used.
        "overlap_later_update_reads_destination",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + h.inner.count })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + h.inner.count })",
        "sumHolders(hs)",
        Some(1),
    ),
];

fn program(setup: &str, body: &str, result: &str, reps: u32) -> String {
    format!(
        "{PRELUDE}
FUNC main() AS Integer
  LET tagEven AS String = \"even-step\"
  LET tagOdd AS String = \"odd\"
{setup}
  MUT i AS Integer = 0
  FOR rep = 1 TO {reps}
    FOR k = 0 TO 63
{body}
      i = i + 1
    NEXT
  NEXT
  io::print(toString({result}))
  RETURN 0
END FUNC
"
    )
}

/// Build with `--debug`, run, and return `(stdout, alloc_calls, free_calls,
/// live_bytes)` summed over the arenas.
fn run(name: &str, source: &str) -> (String, u64, u64, u64) {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name}: build failed:\n{stdout}\n{}\n--- source ---\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .find(|p| p.ends_with("-glibc.out"))
        .or_else(|| {
            stdout
                .lines()
                .find_map(|line| line.strip_prefix("Wrote executable to "))
        })
        .expect("an executable")
        .to_string();
    let out = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let report = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "{name}: program failed:\n{text}\n{report}"
    );
    let sum = |key: &str| -> u64 {
        report
            .lines()
            .filter_map(|line| line.strip_prefix("arena."))
            .filter_map(|rest| rest.split_once(key))
            .filter(|(arena, _)| !arena.contains('.'))
            .map(|(_, n)| n.trim().parse::<u64>().expect("a count"))
            .sum()
    };
    (
        text,
        sum(".alloc_calls "),
        sum(".free_calls "),
        sum(".live_bytes "),
    )
}

#[test]
fn a_nested_record_update_allocates_only_what_its_size_change_needs() {
    let updates = 64 * u64::from(REPS);
    let mut failures = Vec::new();
    for (name, setup, body, contrast, result, allowed) in CASES {
        let (want, _, _, _) = run(
            &format!("b784_{name}_contrast"),
            &program(setup, contrast, result, REPS),
        );
        let (got, alloc, free, live) = run(
            &format!("b784_{name}_1"),
            &program(setup, body, result, REPS),
        );
        let (_, alloc2, free2, live2) = run(
            &format!("b784_{name}_2"),
            &program(setup, body, result, 2 * REPS),
        );
        if got != want {
            failures.push(format!(
                "{name}: printed {got:?}, the contrast spelling {want:?}"
            ));
            continue;
        }
        if alloc != free || live != 0 || alloc2 != free2 || live2 != 0 {
            failures.push(format!(
                "{name}: leaked ({alloc}/{free}/{live} B, {alloc2}/{free2}/{live2} B)"
            ));
            continue;
        }
        let Some(allowed) = allowed else {
            continue;
        };
        let growth = alloc2.saturating_sub(alloc);
        if growth > allowed * updates {
            failures.push(format!(
                "{name}: {alloc} allocations at {updates} updates, {alloc2} at {} — \
                 {:.2} per update ({allowed} allowed)",
                2 * updates,
                growth as f64 / updates as f64
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "bug-784: a nested record update must allocate only what a size change needs:\n  {}",
        failures.join("\n  ")
    );
}

/// A failure after the destination was written would leave the element half
/// updated. Under a function-level `TRAP` that reads the list, the destination
/// variant is not used, and the failed update leaves the element as it was.
#[test]
fn a_later_failure_leaves_the_element_unchanged() {
    let source = format!(
        "{PRELUDE}
FUNC run() AS Integer
  MUT hs AS List OF Holder = []
  LET tagOdd AS String = \"odd\"
  hs = holders()
  hs = collections::set(hs, 0, WITH collections::get(hs, 0) {{ touches := 9223372036854775807 }})
  LET h AS Holder = collections::get(hs, 0)
  hs = collections::set(hs, 0, WITH h {{ inner := stepWide(h.inner, 1, tagOdd), touches := h.touches + 1 }})
  io::print(\"not reached\")
  RETURN 0

  TRAP(e)
    LET q AS Holder = collections::get(hs, 0)
    io::print(toString(q.inner.count) & \"/\" & q.inner.tag & \"/\" & toString(q.touches))
    RETURN 1
  END TRAP
END FUNC

FUNC main() AS Integer
  io::print(\"run -> \" & toString(run()))
  RETURN 0
END FUNC
"
    );
    let (out, alloc, free, live) = run("b784_later_failure", &source);
    assert_eq!(
        out, "0//9223372036854775807\nrun -> 1\n",
        "a failed update must leave the element unchanged"
    );
    assert!(
        alloc == free && live == 0,
        "leaked: {alloc}/{free}/{live} B"
    );
}

/// bug-784: `benchmark/language/mfb/src/recordbench.mfb:test_record_nested`'s shape — the
/// update loop's `LET h = get(hs, k)` and a checksum loop that binds `h` again.
/// The second binding once disqualified the first (the borrow analysis keyed
/// facts by name across the function): 3 allocations per update. Each binding is
/// its own now, so the update loop allocates nothing per update.
fn rebound_program(updates: u32, contrast: bool) -> String {
    let get = |k: &str| {
        if contrast {
            format!("ident(collections::get(hs, {k}))")
        } else {
            format!("collections::get(hs, {k})")
        }
    };
    format!(
        "{PRELUDE}
FUNC main() AS Integer
  LET tagEven AS String = \"even-step\"
  LET tagOdd AS String = \"odd\"
  MUT hs AS List OF Holder = holders()
  FOR i = 0 TO {last}
    LET k AS Integer = i MOD 64
    LET h AS Holder = {g1}
    IF i MOD 2 = 0 THEN
      hs = collections::set(hs, k, WITH h {{ inner := stepWide(h.inner, i, tagEven), touches := h.touches + 1 }})
    ELSE
      hs = collections::set(hs, k, WITH h {{ inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 }})
    END IF
  NEXT
  MUT sumv AS Integer = 0
  FOR k = 0 TO 63
    LET h AS Holder = {g2}
    sumv = (sumv * 31 + h.id + len(h.label) + h.touches + wideSum(h.inner)) MOD 1000000007
  NEXT
  io::print(toString(sumv))
  RETURN 0
END FUNC
",
        last = updates - 1,
        g1 = get("k"),
        g2 = get("k"),
    )
}

#[test]
fn a_rebound_get_binding_keeps_its_element_update() {
    let updates = 64 * REPS;
    let (want, _, _, _) = run("b784_rebound_contrast", &rebound_program(updates, true));
    let (got, alloc, free, live) = run("b784_rebound_1", &rebound_program(updates, false));
    let (got2, alloc2, free2, live2) = run("b784_rebound_2", &rebound_program(2 * updates, false));
    let (want2, _, _, _) = run(
        "b784_rebound_contrast_2",
        &rebound_program(2 * updates, true),
    );
    assert_eq!(
        got, want,
        "the update loop must print what the copying spelling prints"
    );
    assert_eq!(got2, want2);
    assert!(
        alloc == free && live == 0 && alloc2 == free2 && live2 == 0,
        "leaked: {alloc}/{free}/{live} B, {alloc2}/{free2}/{live2} B"
    );
    assert_eq!(
        alloc2,
        alloc,
        "{alloc} allocations at {updates} updates, {alloc2} at {}: a re-bound `h` \
         must not cost its element update an allocation",
        2 * updates
    );
}

/// bug-784, the negative: with `h` bound three times, a binding whose container
/// is written while it is live, and one passed whole to a function, must still
/// be copies — each prints the value it read, not what the container holds later.
#[test]
fn an_unsafe_rebinding_beside_a_borrowing_one_reads_its_own_copy() {
    let source = format!(
        "{PRELUDE}
FUNC touchesOf(h AS Holder) AS Integer
  RETURN h.touches
END FUNC

FUNC main() AS Integer
  MUT hs AS List OF Holder = holders()
  MUT gs AS List OF Holder = holders()
  MUT out AS String = \"\"
  FOR j = 0 TO 2
    LET h AS Holder = collections::get(hs, j)
    out = out & toString(h.inner.count) & \",\"
  NEXT
  FOR j = 0 TO 2
    LET h AS Holder = collections::get(gs, j)
    gs = collections::set(gs, j, Holder[id := 99, label := \"replaced-label\", inner := newWide(50 + j), touches := 7])
    out = out & toString(h.id) & \"/\" & h.label & \"/\" & h.inner.name & \";\"
  NEXT
  FOR j = 0 TO 2
    LET h AS Holder = collections::get(gs, j)
    out = out & toString(touchesOf(h)) & \"+\" & toString(h.id) & \" \"
  NEXT
  IF len(out) > 0 THEN
    LET h AS Holder = collections::get(hs, 1)
    out = out & h.label
  ELSE
    LET h AS Holder = collections::get(gs, 1)
    out = out & h.label
  END IF
  io::print(out)
  RETURN 0
END FUNC
"
    );
    let (out, alloc, free, live) = run("b784_rebound_negative", &source);
    assert_eq!(
        out,
        "0,0,0,0/holder-0/wide-0;1/holder-1/wide-1;2/holder-2/wide-2;7+99 7+99 7+99 holder-1\n"
    );
    assert!(
        alloc == free && live == 0,
        "leaked: {alloc}/{free}/{live} B"
    );
}
