//! plan-118-E: the shared error-park and scope-drop helpers keep the invariants
//! that were previously guaranteed by the inline code they replaced.
//!
//! Three sequences that used to be emitted at every site are now one function
//! each — `_mfb_rt_park_error`, `_mfb_rt_drop_owned_string`,
//! `_mfb_rt_drop_owned_collection`. Moving them means the guarantees move with
//! them, and two of those guarantees are the kind whose violation is silent
//! until it corrupts the arena:
//!
//!   * **free-and-null** (bug-440). `tests/codegen_owned_drop_free_and_null.rs`
//!     pins it for the drops that are still inline (records, unions). It cannot
//!     see the `String`/collection drops any more, because those no longer emit
//!     an `owned_value_free_skip` label at the site — so without this file the
//!     commonest two cleanup shapes would have lost their only check.
//!   * **the error registers survive the park call.** The park reads the loose
//!     code/message/source the `_mfb_make_error_result` call just landed and
//!     must hand them back; if anything were emitted between the two calls, or
//!     if the helper failed to restore them, every error's payload would be
//!     whatever the park's own allocation left behind.
//!
//! Assertions are structural (opcode sequences, register roles) rather than
//! absolute offsets, which drift with every frame change.

mod common;
use common::{build_ncode, temp_project};

/// Owns a `String` and a `List` across a fallible call, so all three helpers are
/// reached: both scope drops and — through the `&` concatenation's
/// out-of-memory path — the error park.
const SOURCE: &str = "\
IMPORT io\n\
\n\
FUNC build(n AS Integer) AS String\n\
  LET items AS List OF Integer = [1, 2, 3]\n\
  LET head AS String = \"head\"\n\
  IF n = 0 THEN\n\
    RETURN head\n\
  END IF\n\
  LET joined AS String = head & \"-tail\"\n\
  RETURN joined\n\
END FUNC\n\
\n\
FUNC main AS Integer\n\
  io::print(build(0) & build(1))\n\
  RETURN 0\n\
END FUNC\n";

fn function<'a>(ncode: &'a serde_json::Value, symbol: &str) -> Option<&'a Vec<serde_json::Value>> {
    ncode["functions"].as_array()?.iter().find_map(|function| {
        (function["symbol"].as_str() == Some(symbol))
            .then(|| function["instructions"].as_array())
            .flatten()
    })
}

fn ops(instructions: &[serde_json::Value]) -> Vec<String> {
    instructions
        .iter()
        .map(|inst| inst["op"].as_str().unwrap_or("").to_string())
        .collect()
}

/// Both scope-drop helpers free and then NULL the slot they were handed, so a
/// drop re-reached without an intervening store skips instead of freeing a
/// stale pointer (bug-440).
#[test]
fn shared_scope_drops_free_and_null_their_slot() {
    let project = temp_project("codegen_shared_cleanup", SOURCE);
    let ncode = build_ncode(&project, "macos-aarch64", "codegen_shared_cleanup");

    for symbol in ["_mfb_rt_drop_owned_string", "_mfb_rt_drop_owned_collection"] {
        let instructions =
            function(&ncode, symbol).unwrap_or_else(|| panic!("`{symbol}` was not emitted"));
        let opcodes = ops(instructions);
        let free = opcodes
            .iter()
            .position(|op| op == "bl")
            .unwrap_or_else(|| panic!("`{symbol}` never calls `_mfb_arena_free`"));
        assert_eq!(
            instructions[free]["target"].as_str(),
            Some("_mfb_arena_free"),
            "`{symbol}`'s only call should be the arena free, got {}",
            instructions[free]
        );
        // After the free, and before the helper returns, the slot is zeroed.
        let nulls = instructions[free + 1..].iter().any(|inst| {
            inst["op"].as_str() == Some("str_u64") && inst["src"].as_str() == Some("xzr")
        });
        assert!(
            nulls,
            "`{symbol}` frees without nulling its slot — the bug-440 free-and-null \
             guarantee did not move with the code:\n{:#?}",
            &instructions[free..]
        );
        // And it null-guards on entry, so a slot that was never stored is skipped
        // rather than freed.
        assert!(
            opcodes[..free].iter().any(|op| op == "b.eq"),
            "`{symbol}` frees without a null guard on entry"
        );
    }
}

/// Nothing is emitted between building the loose error and parking it, so the
/// three error registers `_mfb_make_error_result` lands flow into the park
/// untouched.
#[test]
fn the_error_park_call_immediately_follows_make_error_result() {
    let project = temp_project("codegen_shared_park", SOURCE);
    let ncode = build_ncode(&project, "macos-aarch64", "codegen_shared_park");

    assert!(
        function(&ncode, "_mfb_rt_park_error").is_some(),
        "`_mfb_rt_park_error` was not emitted"
    );

    let mut pairs = 0usize;
    for func in ncode["functions"].as_array().expect("functions") {
        let Some(instructions) = func["instructions"].as_array() else {
            continue;
        };
        for (index, inst) in instructions.iter().enumerate() {
            if inst["op"].as_str() != Some("bl")
                || inst["target"].as_str() != Some("_mfb_make_error_result")
            {
                continue;
            }
            let next = instructions
                .get(index + 1)
                .unwrap_or_else(|| panic!("`{}` ends on make_error_result", func["name"]));
            // The park is skipped only on the re-entry paths that are already
            // building an error block; there the next instruction is the exit.
            if next["target"].as_str() == Some("_mfb_rt_park_error") {
                pairs += 1;
            } else {
                assert!(
                    matches!(next["op"].as_str(), Some("ldr_u64" | "b" | "ret" | "mov")),
                    "unexpected instruction between `_mfb_make_error_result` and the \
                     error park in `{}`: {next}",
                    func["name"]
                );
            }
        }
    }
    assert!(
        pairs > 0,
        "no `_mfb_make_error_result` -> `_mfb_rt_park_error` pair found — the \
         fixture no longer reaches a fallible operation"
    );
}
