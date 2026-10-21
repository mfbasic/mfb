//! Two call shapes of `engine/builder/builder_emit_helpers.rs` no other suite
//! lowers in process.
//!
//! * plan-120-E: an indirect call through a `FUNC … AS Nothing` value still checks
//!   the callee's error tag (`call_value_ok`) — a `Nothing` callee can `FAIL` like
//!   any other, and without the check its error code would be read as no value.
//! * plan-158-A: an inline `TRAP` on an `audio` helper, whose errors may come back
//!   PARKED, re-stamps a parked error with this call site before the trap adopts
//!   it (`raw_helper_error_not_parked`).
//!
//! The program is `tests/runtime/rt_audio_mml_bounds.rs`'s, plus the indirect call.

use super::lowered::labels_on;
use crate::testutil::CodeTarget;

const SOURCE: &str = "\
IMPORT audio
IMPORT io

FUNC shout(v AS Integer) AS Nothing
  io::print(toString(v))
END FUNC

FUNC main() AS Integer
  LET f AS FUNC(Integer) AS Nothing = shout
  f(1)
  RES out AS audio::AudioOutput = audio::openOutput(48000, 1, 512) TRAP(e)
    io::print(\"no output device: \" & e.message)
    RETURN 0
  END TRAP
  audio::play(out, \"c\") TRAP(e)
    io::print(\"raised \" & e.message)
    audio::close(out)
    RETURN 0
  END TRAP
  audio::close(out)
  RETURN 0
END FUNC
";

#[test]
fn an_indirect_nothing_call_checks_its_tag_and_a_trapped_audio_call_restamps() {
    for target in CodeTarget::ALL {
        let stems = labels_on(SOURCE, "main", target);
        for stem in ["call_value_ok", "raw_helper_error_not_parked"] {
            assert!(
                stems.iter().any(|s| s == stem),
                "{}: no `{stem}` in main: {stems:?}",
                target.name()
            );
        }
    }
}
