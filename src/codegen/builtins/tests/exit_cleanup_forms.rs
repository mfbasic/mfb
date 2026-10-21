//! A loop exit releases what the loop body owns (`engine/control/builder_exits.rs`,
//! `emit_cleanup_branch_to_depth`).
//!
//! `EXIT FOR` / `CONTINUE FOR` branch past the end of the body, so every cleanup
//! the body registered runs on the branch as well as at the body's end: a
//! resource-union binding's tag-dispatched close, and a resource-owning list's
//! drain. Each shape is lowered with and without the exit statement; the exit
//! adds exactly one more copy of that cleanup to `main`.

use super::lowered::labels_on;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

const HEAD: &str = "\
IMPORT collections
IMPORT fs
IMPORT io
IMPORT tcp

UNION Stream
  fs::File
  tcp::Socket
END UNION

FUNC mkFiles() AS List OF RES fs::File
  RES f AS fs::File = fs::createTempFile()
  RETURN [f]
END FUNC
";

fn program(bind: &str, exit: &str) -> String {
    format!(
        "{HEAD}\nFUNC main() AS Integer\n  FOR i = 1 TO 3\n{bind}\n{exit}\n    io::print(toString(i))\n  NEXT\n  RETURN 0\nEND FUNC\n"
    )
}

/// How many labels of `main` start with `stem`.
fn count(source: &str, stem: &str, target: CodeTarget) -> usize {
    crate::codegen::engine::tests::test_support::Stream::of(code_function(
        code_for_src_cached(source, target, Console),
        "main",
    ))
    .labels()
    .iter()
    .filter(|(_, label)| label.starts_with(stem))
    .count()
}

#[test]
fn an_exit_closes_a_resource_union_the_body_owns() {
    let bind = "    RES s AS Stream = fs::createTempFile()";
    let exit = "    IF i = 2 THEN\n      EXIT FOR\n    END IF";
    for target in CodeTarget::ALL {
        let with_exit = count(&program(bind, exit), "resource_union_drop_done", target);
        let without = count(&program(bind, ""), "resource_union_drop_done", target);
        assert_eq!(with_exit, without + 1, "{}", target.name());
    }
}

#[test]
fn a_continue_drains_a_resource_list_the_body_owns() {
    let bind = "    LET files AS List OF RES fs::File = mkFiles()";
    let exit = "    IF i = 2 THEN\n      CONTINUE FOR\n    END IF";
    for target in CodeTarget::ALL {
        let with_exit = count(&program(bind, exit), "owned_list_close_ok", target);
        let without = count(&program(bind, ""), "owned_list_close_ok", target);
        assert_eq!(with_exit, without + 1, "{}", target.name());
        // The drain is the owned-list route, not a per-element resource close.
        assert!(labels_on(&program(bind, exit), "main", target)
            .iter()
            .any(|stem| stem == "owned_list_drain_loop"));
    }
}
