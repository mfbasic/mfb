//! Resource-union aliases (`builder_control/bind_op.rs`).
//!
//! `RES s AS Stream STATE Cursor = f` wraps the live `fs::File` `f` in a union box
//! without taking the record: `f` still closes it. The `STATE` the alias attaches
//! rides that record, so `f`'s drop must free it after its close — the union's
//! own drop frees only its box. Observed through `main`'s labels: the owner's
//! state free (`resource_state_free_skip`) is emitted only when there is a
//! `STATE` to free.

use crate::codegen::engine::tests::test_support::Stream;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

fn program(state: &str, use_state: &str) -> String {
    format!(
        "IMPORT fs
IMPORT tcp
IMPORT io

TYPE Cursor
  pos AS Integer
END TYPE

UNION Stream
  fs::File
  tcp::Socket
END UNION

FUNC main AS Integer
  RES f AS fs::File = fs::createTempFile()
  RES s AS Stream{state} = f
{use_state}  io::print(toString(fs::size(f)))
  RETURN 0
END FUNC
"
    )
}

fn frees_state(source: &str, target: CodeTarget) -> bool {
    Stream::of(code_function(
        code_for_src_cached(source, target, Console),
        "main",
    ))
    .labels()
    .iter()
    .any(|(_, label)| label.starts_with("resource_state_free_skip"))
}

#[test]
fn a_union_alias_hands_its_state_to_the_wrapped_records_owner() {
    let with_state = program(" STATE Cursor", "  s.state.pos = 5\n");
    for target in CodeTarget::ALL {
        assert!(
            frees_state(&with_state, target),
            "{}: f's drop frees the STATE the alias attached",
            target.name()
        );
    }
    assert!(!frees_state(&program("", ""), CodeTarget::LinuxX86_64));
}
