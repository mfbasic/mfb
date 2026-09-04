//! `term::getForeground` must store all four `color::Color` fields, at 0/8/16/24,
//! with alpha the immediate `255`.
//!
//! plan-122-F widened the record `emit_get_color` allocates from the retired
//! 3-field `TermColor` to the 4-field `color::Color`. Three things can go wrong in
//! that emitter and none of them is visible to a black-box fixture:
//!
//!  * a **wrong offset** still reads a plausible byte. The three colour channels are
//!    each masked to `0..255` before the store, so a store landing at 8 instead of
//!    16 yields another channel's value — a real colour, just not the right one, and
//!    only for inputs where the two channels differ.
//!  * a **missing alpha store** leaves whatever the arena handed back at offset 24.
//!    A freshly-zeroed block reads `0`, so `getForeground().alpha` would be `0` and
//!    look like a deliberate "transparent", not like uninitialised memory.
//!  * an **alpha unpacked from the state slot** rather than fixed. The slot is only
//!    `0xBBGGRR`, so bits 24+ are zero and the result is again a plausible `0`.
//!
//! Each of those passes `getForeground().red == 1` and any test that only samples
//! one channel. Reading the emitted plan is what makes the offsets themselves the
//! assertion.
//!
//! The companion behavioural test is
//! `tests/rt-behavior/term/func_term_color_roundtrip_valid`, which proves the values
//! survive a real `setForeground`/`getForeground` round trip on this host. Neither
//! subsumes the other: that one cannot see a wrong offset that happens to read a
//! plausible byte, and this one cannot see the runtime.

mod common;

use serde_json::Value;
use std::process::Command;

const SOURCE: &str = r#"IMPORT term
IMPORT color
IMPORT io

FUNC main AS Integer
  term::on()
  term::setForeground(color::rgb(1, 2, 3))
  LET c AS color::Color = term::getForeground()
  term::off()
  io::print(toString(c.red) & toString(c.alpha))
  RETURN 0
END FUNC
"#;

fn ncode(name: &str, target: &str) -> Value {
    let project = common::temp_project(name, SOURCE);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-ncode")
        .arg("-target")
        .arg(target)
        .arg(&project)
        .output()
        .expect("run mfb build -ncode");
    assert!(
        output.status.success(),
        "mfb build -ncode -target {target} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let text =
        std::fs::read_to_string(project.join(format!("{name}.ncode"))).expect("read ncode dump");
    let plan: Value = serde_json::from_str(&text).expect("parse ncode json");
    let _ = std::fs::remove_dir_all(&project);
    plan
}

/// The record-field `str_u64`s in the function whose symbol contains `needle`, as
/// `(offset, base)` pairs.
///
/// Two shapes have to be handled and getting either wrong makes the test vacuous.
/// `offset` is a JSON **string** (`"16"`), not a number — reading it as a number
/// yields `None` for every instruction and the assertion then compares sentinels.
/// And stack spills must be excluded, since the allocated record's pointer lives in
/// a general register: the stack pointer is spelled **per target** (`sp` on
/// AArch64, `rsp` on x86-64), so filtering only `sp` leaves eight x86-64 spills in
/// the result and the count assertion fails on a target that is actually correct.
fn record_stores(plan: &Value, needle: &str) -> Vec<(i64, String)> {
    const STACK_BASES: [&str; 4] = ["sp", "rsp", "rbp", "x29"];
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter(|f| f["symbol"].as_str().is_some_and(|s| s.contains(needle)))
        .flat_map(|f| f["instructions"].as_array().expect("instructions").iter())
        .filter(|i| {
            i["op"] == "str_u64" && !i["base"].as_str().is_some_and(|b| STACK_BASES.contains(&b))
        })
        .filter_map(|i| {
            let offset = i["offset"].as_str()?.parse::<i64>().ok()?;
            Some((offset, i["base"].as_str().unwrap_or_default().to_string()))
        })
        .collect()
}

/// The record `getForeground` allocates carries four fields at 0/8/16/24.
///
/// Asserted on two targets because `emit_get_color` is shared but the register
/// names it emits are per-target; an offset regression would be identical on both,
/// while a target-specific one would show on exactly one.
#[test]
fn get_foreground_stores_all_four_color_fields() {
    for target in ["macos-aarch64", "linux-x86_64"] {
        let plan = ncode("term_color_offsets", target);
        let found = record_stores(&plan, "getForeground");

        for want in [0i64, 8, 16, 24] {
            assert!(
                found.iter().any(|(off, _)| *off == want),
                "{target}: term::getForeground must store a color::Color field at \
                 offset {want}; a missing store leaves the arena's bytes there, which \
                 read as a plausible 0 rather than as uninitialised. Record stores \
                 found: {found:?}"
            );
        }

        // Exactly four, so a fifth stray store into the record is caught too — the
        // record is 32 bytes and a store past offset 24 would run off it.
        assert_eq!(
            found.len(),
            4,
            "{target}: expected exactly four record-field stores (0/8/16/24), got \
             {found:?}"
        );
    }
}

/// The value stored at offset 24 is the immediate `255`, traced by dataflow.
///
/// The term state slot is `0xBBGGRR` — it has no alpha bits — so any attempt to
/// derive alpha from it yields `0`. That is exactly the value a caller would read as
/// "fully transparent", so the failure is silent.
///
/// **This deliberately does not just look for a `mov_imm 255` anywhere.** `255` is
/// also the per-channel AND mask, emitted three lines earlier, so a bare "does the
/// body contain 255" assertion passes even when the alpha store is missing
/// entirely — vacuous against the exact regression it exists to catch. Instead it
/// walks the body in order, tracks the last writer of every register, finds the
/// store at offset 24, and requires that store's source register to have been last
/// written by a `mov_imm` of `255`.
#[test]
fn get_foreground_alpha_is_the_immediate_255() {
    for target in ["macos-aarch64", "linux-x86_64"] {
        let plan = ncode("term_color_alpha", target);
        let body: Vec<&Value> = plan["functions"]
            .as_array()
            .expect("functions array")
            .iter()
            .filter(|f| {
                f["symbol"]
                    .as_str()
                    .is_some_and(|s| s.contains("getForeground"))
            })
            .flat_map(|f| f["instructions"].as_array().expect("instructions").iter())
            .collect();

        // register -> the immediate it last held, when its last writer was a mov_imm.
        let mut immediates: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut verdict: Option<String> = None;

        for i in &body {
            let op = i["op"].as_str().unwrap_or_default();
            if op == "str_u64"
                && i["offset"].as_str() == Some("24")
                && !i["base"]
                    .as_str()
                    .is_some_and(|b| ["sp", "rsp", "rbp", "x29"].contains(&b))
            {
                let src = i["src"].as_str().unwrap_or_default();
                verdict = Some(
                    immediates
                        .get(src)
                        .cloned()
                        .unwrap_or_else(|| format!("<{src} was not last written by a mov_imm>")),
                );
                break;
            }
            // Track writes. A mov_imm records its value; any other write to a
            // register invalidates what we knew about it.
            if let Some(dst) = i["dst"].as_str() {
                if op == "mov_imm" {
                    if let Some(v) = i["value"].as_str() {
                        immediates.insert(dst.to_string(), v.to_string());
                        continue;
                    }
                }
                immediates.remove(dst);
            }
        }

        assert_eq!(
            verdict.as_deref(),
            Some("255"),
            "{target}: the color::Color field at offset 24 (alpha) must be stored from \
             a register holding the immediate 255. A terminal cell has no alpha \
             channel, and the 0xBBGGRR state slot has no alpha bits to read, so \
             anything derived from the slot would be 0 and read as 'transparent'. \
             Emitted body:\n{}",
            serde_json::to_string_pretty(&body).unwrap_or_default(),
        );
    }
}
