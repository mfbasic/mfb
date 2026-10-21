//! Shapes whose lowering depends on the BACKEND, lowered for the backend that takes
//! the branch (a lowering for an explicit `CodeTarget` is host-independent).
//!
//! * A record with more fields than the eight argument registers is built by a
//!   helper that takes the tail on the stack (`construct_helpers`): the four fields
//!   past the eighth are loaded from the incoming stack area and stored to the
//!   outgoing one at the call.
//! * A `tls::Listener` sent to a worker copies its backend-specific live slots into
//!   the receiver's arena: macOS holds the bound host as a NUL-terminated arena
//!   string (`ArenaCString`, measured with a `strlen` loop), Windows holds an SSPI
//!   work block (`ArenaBlock`, a fixed-size copy). Linux holds neither.

use crate::codegen::engine::tests::test_support::Stream;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

/// Three construction sites of one record type: the threshold at which a
/// `construct.Wide` helper is synthesized instead of inlining each.
const WIDE_RECORD: &str = "\
IMPORT io

TYPE Wide
  a AS Integer
  b AS Integer
  c AS Integer
  d AS Integer
  e AS Integer
  f AS Integer
  g AS Integer
  h AS Integer
  i AS Integer
  j AS Integer
  k AS String
  l AS Float
END TYPE

FUNC make(n AS Integer) AS Wide
  RETURN Wide[n, n + 1, n + 2, n + 3, n + 4, n + 5, n + 6, n + 7, n + 8, n + 9, \"k\", 1.5]
END FUNC

FUNC main() AS Integer
  LET w AS Wide = make(1)
  LET w2 AS Wide = Wide[a := 1, b := 2, c := 3, d := 4, e := 5, f := 6, g := 7, h := 8, i := 9, j := 10, k := \"x\", l := 2.5]
  LET w3 AS Wide = Wide[a := 3, b := 3, c := 3, d := 3, e := 3, f := 3, g := 3, h := 3, i := 3, j := 3, k := \"z\", l := 3.5]
  io::print(toString(w.j + w2.a + w3.a))
  RETURN 0
END FUNC
";

const LISTENER_TRANSFER: &str = "\
IMPORT thread
IMPORT tls
IMPORT io

ISOLATED FUNC serve(w AS ThreadWorker OF Nothing RES tls::Listener TO Integer, n AS Integer) AS Integer
  RES l AS tls::Listener = thread::accept(w)
  RETURN n
END FUNC

FUNC main() AS Integer
  RES l AS tls::Listener = tls::listen(\"127.0.0.1\", 0, \"cert.pem\", \"key.pem\")
  RES t AS Thread OF Nothing RES tls::Listener TO Integer = thread::start(serve, 1)
  thread::transfer(t, l)
  io::print(toString(thread::waitFor(t)))
  RETURN 0
END FUNC
";

#[test]
fn a_record_wider_than_the_argument_registers_is_built_by_one_shared_helper() {
    for target in CodeTarget::ALL {
        let plan = code_for_src_cached(WIDE_RECORD, target, Console);
        // Defined once...
        code_function(plan, "construct.Wide");
        // ...and called from each of the three construction sites.
        let calls: usize = ["main", "make"]
            .iter()
            .map(|name| {
                let stream = Stream::of(code_function(plan, name));
                stream
                    .calls_between(0, stream.instructions.len())
                    .iter()
                    .filter(|callee| callee.as_str() == "_mfb_ctor_Wide")
                    .count()
            })
            .sum();
        assert_eq!(
            calls,
            3,
            "{}: the three `Wide[...]` sites share the twelve-argument helper, whose last \
             four arguments travel on the stack",
            target.name()
        );
    }
}

#[test]
fn a_listener_copies_the_slots_its_backend_keeps_in_the_arena() {
    let copy = |target| {
        let plan = code_for_src_cached(LISTENER_TRANSFER, target, Console);
        // The copier exists whether or not the backend has a slot for it to copy.
        plan.functions
            .iter()
            .filter(|function| function.name == "thread_copy_value.tls.Listener")
            .flat_map(|function| Stream::of(function).labels())
            .map(|(_, name)| name)
            .filter(|name| name.starts_with("thread_copy_res_slot"))
            .collect::<Vec<_>>()
    };
    let macos = copy(CodeTarget::MacosAarch64);
    assert!(
        macos.iter().any(|name| name.contains("_strlen")),
        "macOS keeps the bound host as an arena C string: the copy measures it with a \
         strlen loop first: {macos:?}"
    );
    let windows = copy(CodeTarget::WindowsX86_64);
    assert!(
        !windows.is_empty() && !windows.iter().any(|name| name.contains("_strlen")),
        "Windows keeps an SSPI work block of fixed size in the arena and copies it \
         without measuring: {windows:?}"
    );
    let linux = copy(CodeTarget::LinuxX86_64);
    assert!(
        linux.is_empty(),
        "Linux keeps no arena-resident listener slot, so there is nothing to copy: {linux:?}"
    );
}

const HOST_PATHS: &str = "\
IMPORT fs
IMPORT io
IMPORT os

FUNC main() AS Integer
  MUT r AS String = \"assets\"
  MUT d AS String = \"data\"
  MUT n AS String = \"a/b/../c\"
  r = os::appResourcePath(r)
  r = fs::pathBaseName(r)
  d = os::appDataPath(d)
  d = fs::pathBaseName(d)
  n = fs::pathNormalize(n)
  io::print(r & d & n)
  RETURN 0
END FUNC
";

/// `os::appResourcePath(x)` and `fs::pathNormalize(x)` as self-updates scan for the
/// HOST's path separator: `/` everywhere, and `\` as well on Windows - the arm must
/// agree with the copying helper exactly, or `a\..\b` is accepted on one side and
/// raises `ErrInvalidPath` on the other.
#[test]
fn the_host_path_self_updates_scan_for_the_windows_separator_only_on_windows() {
    let backslashes = |target: CodeTarget| {
        let plan = code_for_src_cached(HOST_PATHS, target, Console);
        code_function(plan, "main")
            .instructions
            .iter()
            .filter(|i| {
                i.op == crate::arch::ops::CodeOp::CmpImm && i.get("rhs").as_deref() == Some("92")
            })
            .count()
    };
    assert!(
        backslashes(CodeTarget::WindowsX86_64) > 0,
        "Windows paths separate on `\\` (92) too"
    );
    assert_eq!(
        backslashes(CodeTarget::LinuxX86_64),
        0,
        "POSIX paths separate only on `/`: a `\\` is an ordinary byte there"
    );
}

/// An app build joins the module's resource suffix (`Resources` in a macOS bundle,
/// `share/<module>` in a Linux AppImage) onto the executable's directory, as
/// immediates written after the directory bytes, so its `main` is longer than the
/// console build's. (A Windows app `.exe` sits beside its resources like a console
/// build: no suffix.)
#[test]
fn an_app_build_joins_the_resource_suffix_onto_the_executable_directory() {
    use crate::testutil::code_for_src_mode;
    let length = |plan: &crate::codegen::engine::types::NativeCodePlan| {
        code_function(plan, "main").instructions.len()
    };
    for target in [CodeTarget::MacosAarch64, CodeTarget::LinuxX86_64] {
        let console = code_for_src_mode(HOST_PATHS, target, Console);
        let app = code_for_src_mode(
            HOST_PATHS,
            target,
            target.app_mode().expect("this backend has an app mode"),
        );
        assert!(
            length(&app) > length(&console),
            "{}: the app build's `appResourcePath` writes the suffix and its trailing `/`",
            target.name()
        );
    }
}
