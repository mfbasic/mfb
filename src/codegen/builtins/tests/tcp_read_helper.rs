//! The socket read helper builds either a byte list or a validated `String`.
//!
//! `lower_net_read_helper` serves `tcp::read` (a `List OF Byte`); its `text` arm builds
//! a `String` instead — the allocation of `{len, bytes, NUL}`, the UTF-8 validation
//! and, when the bytes are not UTF-8, the release of BOTH the half-built string and
//! the read buffer before `ErrEncoding` is raised. No member lowers the text form
//! today (`tcp::read` is the only caller), so a program cannot reach it; the helper
//! is driven directly, on every backend, so the arm does not rot unseen.
//!
//! What distinguishes the two builds is exactly the encoding exit: only the text
//! build has an `_encoding_error` label, and it frees the string (the read length
//! plus the 9 bytes of header and terminator) before the read buffer.

use std::collections::HashMap;

use super::platforms::platform_for;
use crate::arch::ops::CodeOp;
use crate::codegen::engine::mir;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, CodeTarget};

const READS_A_SOCKET: &str = "\
IMPORT tcp
IMPORT io

FUNC main() AS Integer
  RES sock AS tcp::Socket = tcp::connect(\"127.0.0.1\", 9)
  LET bytes = tcp::read(sock, 16)
  io::print(toString(len(bytes)))
  RETURN 0
END FUNC
";

fn labels_of(instructions: &[crate::codegen::engine::types::CodeInstruction]) -> Vec<String> {
    instructions
        .iter()
        .filter(|i| i.op == CodeOp::Label)
        .filter_map(|i| i.get("name"))
        .collect()
}

#[test]
fn the_text_build_validates_and_the_byte_build_does_not() {
    for target in CodeTarget::ALL {
        let plan = code_for_src_cached(READS_A_SOCKET, target, Console);
        let imports: HashMap<String, String> = plan
            .imports
            .iter()
            .map(|import| (import.symbol.clone(), import.library.clone()))
            .collect();
        let platform = platform_for(target);
        mir::set_backend(platform.backend());
        let lower = |text| {
            crate::codegen::builtins::tcp::gen_io::lower_net_read_helper(
                "rd", &imports, &*platform, text,
            )
            .unwrap_or_else(|err| {
                panic!("the read helper (text={text}) on {}: {err}", target.name())
            })
        };
        let (bytes, _, _) = lower(false);
        let (text, _, _) = lower(true);
        let bytes_labels = labels_of(&bytes);
        let text_labels = labels_of(&text);
        assert!(
            !bytes_labels.iter().any(|l| l == "rd_encoding_error"),
            "{}: a byte list has no encoding to refuse",
            target.name()
        );
        assert!(
            text_labels.iter().any(|l| l == "rd_encoding_error"),
            "{}: a String read must have an exit for bytes that are not UTF-8",
            target.name()
        );
        assert!(
            text_labels.iter().any(|l| l == "rd_str_copy")
                && text_labels.iter().any(|l| l == "rd_str_done"),
            "{}: the text build copies the bytes into a String block",
            target.name()
        );
        assert!(
            bytes_labels.iter().any(|l| l == "rd_entry_loop"),
            "{}: the byte build copies the bytes into list entries",
            target.name()
        );
    }
}
