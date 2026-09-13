//! bug-470: every script that rewrites fixture dumps INSIDE the tree must take
//! the tree's gate lock.
//!
//! The filed bug was "`artifact-gate.sh` and `test-accept.sh` do not lock
//! against each other", and the fix as first written locked exactly those two
//! plus `sync-goldens.sh`. But the contended resource named in the bug doc is
//! not those scripts — it is
//! `tests/<fixture>/<pkg>.{ast,ir,hex,nir,nplan,nobj,ncode,mir}`, "written by
//! both and deleted by one". Three MORE scripts wrote and deleted those same
//! paths: two golden-regeneration scripts (merged by plan-131-B into
//! `regen-native-goldens.sh`) and the since-deleted bug-387 byte-identity gate.
//!
//! Regenerate-then-gate is the normal workflow after an intended codegen change,
//! so a `regen-*` running beside an `artifact-gate` in one tree is a realistic
//! pairing, not an exotic one — and it corrupts in exactly the way the filed bug
//! describes.
//!
//! This test exists because "the list of scripts that lock" and "the list of
//! scripts that write into the tree" were two lists that had already drifted
//! apart once. It derives the second list from the source rather than restating
//! it, so a new writer added later fails here instead of silently joining the
//! race.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every script that emits a codegen dump must be classified exactly once,
/// here. `true` = it takes the tree's gate lock; `false` = it is exempt, and the
/// string says why.
///
/// A CLASSIFICATION, not a recogniser. The first two attempts at this test used
/// a heuristic ("does it `rm -f` a dump beside a fixture?") and both were wrong
/// in the same direction — they under-reported, which is the direction that
/// silently ships the bug. The first missed the raw-golden regeneration script
/// (since merged into `regen-native-goldens.sh`); the second
/// missed `bench-lowering.sh`, which clears its dumps with `find … -delete`
/// rather than `rm -f`. And the exemption list written alongside them was wrong
/// too: the host-only `ncode-determinism` script (since folded into
/// `ncode-determinism-alltargets.sh`) looked temp-only because it opens a `mktemp`, but
/// that file is only its hash accumulator — the build itself writes to
/// `$REPO/$td`, beside the fixture.
///
/// So the rule here is exhaustiveness, not pattern-matching: a script that emits
/// a dump and is absent from this table fails the test, and whoever adds it has
/// to state which side it is on.
const CLASSIFICATION: &[(&str, bool, &str)] = &[
    ("scripts/artifact-gate.sh", true, "the gate itself"),
    ("scripts/test-accept.sh", true, "the acceptance harness"),
    (
        "scripts/sync-goldens.sh",
        true,
        "spawns test-accept.sh, then copies goldens; holds across both",
    ),
    (
        "scripts/regen-native-goldens.sh",
        true,
        "rm -f \"$td/$pkg\".{nir,nplan,nobj,ncode,mir}, then rebuilds them beside the fixture",
    ),
    (
        "scripts/ncode-determinism-alltargets.sh",
        true,
        "builds -ncode into $REPO/$td for every target",
    ),
    (
        "tools/bench-lowering/bench-lowering.sh",
        true,
        "deletes the probe's *.ncode with `find -delete`, then cold-builds it",
    ),
    (
        "scripts/diag-set-diff.sh",
        true,
        "replays each golden's own `mfb build` against the fixture dir",
    ),
    (
        "scripts/artifact-baseline.sh",
        false,
        "copies each fixture to $WORKDIR/w$slot and builds THERE, never in-tree",
    ),
];

/// A script emits a codegen dump when it passes one of the dump flags to `mfb`.
/// This decides only WHICH scripts must be classified — not what the answer is.
fn emits_a_codegen_dump(text: &str) -> bool {
    const FLAGS: &[&str] = &[
        "-ncode", "-nir", "-nplan", "-nobj", "-mir", "-ast", "$DUMPS",
    ];
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .any(|l| FLAGS.iter().any(|f| l.contains(f)))
}

fn takes_the_lock(text: &str) -> bool {
    text.contains("gate-lock.sh") && text.contains("gate_lock_acquire")
}

#[test]
fn every_dump_emitting_script_is_classified_and_matches_its_classification() {
    let root = repo_root();
    let mut unclassified = Vec::new();
    let mut wrong = Vec::new();
    let mut seen = Vec::new();

    // `scripts/*.sh` and `tools/*/*.sh` (plan-131-C moved benchmark and generator
    // tooling under `tools/`): a script cannot leave the census by moving there.
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(root.join("scripts"))
        .expect("read scripts/")
        .map(|e| e.expect("dir entry").path())
        .collect();
    for dir in std::fs::read_dir(root.join("tools")).expect("read tools/") {
        let dir = dir.expect("dir entry").path();
        if dir.is_dir() {
            candidates.extend(
                std::fs::read_dir(&dir)
                    .expect("read tools/<dir>")
                    .map(|e| e.expect("dir entry").path()),
            );
        }
    }
    candidates.sort();

    for path in candidates {
        if path.extension().and_then(|e| e.to_str()) != Some("sh") {
            continue;
        }
        let name = path
            .strip_prefix(&root)
            .expect("under the repo root")
            .to_string_lossy()
            .replace('\\', "/");
        if name == "scripts/gate-lock.sh" || name == "scripts/artifact-kinds.sh" {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read script");
        if !emits_a_codegen_dump(&text) {
            continue;
        }
        seen.push(name.clone());
        match CLASSIFICATION.iter().find(|(n, ..)| *n == name) {
            None => unclassified.push(name),
            Some((_, should_lock, why)) => {
                if takes_the_lock(&text) != *should_lock {
                    wrong.push(format!(
                        "{name}: classified {} ({why}) but the script says otherwise",
                        if *should_lock { "locking" } else { "exempt" }
                    ));
                }
            }
        }
    }

    assert!(
        unclassified.is_empty(),
        "these scripts emit a codegen dump and are not in CLASSIFICATION. If a \
         script builds a fixture INSIDE the tree it must take the gate lock, or \
         it races artifact-gate.sh / test-accept.sh exactly as bug-470 \
         describes; if it builds into a scratch dir, add it as exempt WITH the \
         reason: {unclassified:?}"
    );

    // Every classified script is checked, INCLUDING the ones the flag scan
    // cannot see. Three contend without naming a dump flag themselves:
    // `sync-goldens.sh` spawns `test-accept.sh`, `regen-native-goldens.sh` builds
    // `-$ext` from `artifact-kinds.sh`, and `diag-set-diff.sh` replays the argv
    // recorded in each golden's own `$ mfb build …` line. A scan-only test would
    // silently skip all three.
    for (name, should_lock, why) in CLASSIFICATION {
        let text = std::fs::read_to_string(root.join(name))
            .unwrap_or_else(|e| panic!("classified script {name} is missing: {e}"));
        if takes_the_lock(&text) != *should_lock {
            wrong.push(format!(
                "{name}: classified {} ({why}) but the script says otherwise",
                if *should_lock { "locking" } else { "exempt" }
            ));
        }
    }
    wrong.sort();
    wrong.dedup();
    assert!(wrong.is_empty(), "{wrong:#?}");

    // Guard against the scan going blind: if `emits_a_codegen_dump` stops
    // matching, `unclassified` is trivially empty and this test reports a clean
    // sweep over nothing. Five of the eight classified scripts name a dump flag
    // directly; the other three are listed in the comment above. (plan-131-A
    // deleted two flag-naming scripts, the bug-387 gate and the host-only
    // ncode-determinism, so the measured count fell from nine to seven;
    // plan-131-B merged the two flag-naming `.ncode` regeneration scripts into
    // `regen-native-goldens.sh`, which builds `-$ext` and so names none, and the
    // measured count fell to 5.)
    const SCAN_FLOOR: usize = 5;
    assert!(
        seen.len() >= SCAN_FLOOR,
        "the dump-flag scan found only {} script(s), below the known floor of \
         {SCAN_FLOOR}; it has stopped recognising the flags, so the \
         exhaustiveness half of this test is checking nothing",
        seen.len()
    );
}
