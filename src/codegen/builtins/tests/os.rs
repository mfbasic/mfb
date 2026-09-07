//! Codegen contracts for the per-platform `os::` emitters, on **every** backend.
//!
//! `os::version`, `os::uptime` and `os::isAdmin` each `match ctx.platform.family()`
//! and reach a completely different syscall per family. A test that only lowers for
//! the host therefore exercises one arm of three and leaves the others at zero on
//! whichever machine runs it — which is exactly the state CI is in: coverage runs
//! on ubuntu, so the macOS and Windows arms of these files have never been
//! measured. Every case below iterates [`CodeTarget::ALL`], so the arms are covered
//! from any host and the *cross-platform* claims (same result shape everywhere,
//! opposite privilege senses) become checkable at all.
//!
//! Numeric answers are pinned behaviourally elsewhere
//! (`tests/rt-behavior/os/func_os_system_status_valid`) — but only on the one
//! platform that fixture runs on, which is the gap these fill.

use crate::arch::ops::CodeOp;
use crate::codegen::builtins::os;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::engine::tests::test_support::{BuilderHarness, Stream};
use crate::codegen::engine::types::{CodeFunction, CodegenPlatform, NativeCodePlan};
use crate::codegen::registry::AbiCtx;
use crate::os::linux::flavor::LinuxFlavor;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, try_code_for_src, CodeTarget};

/// A program that calls the WHOLE `os::` surface.
///
/// Eighteen of the nineteen members, not a sample: each one dispatches on
/// `ctx.platform.family()` into a different syscall, so a member the program
/// does not call leaves three arms unmeasured rather than one. Calling them all
/// and lowering for all five backends is what turns "the host's arm" into "every
/// arm". `os::resourcePath` is the one left out, because it does not lower for
/// Windows at all -- that refusal is a contract of its own and gets its own case
/// below.
const SRC: &str = "\
IMPORT io
IMPORT os

FUNC main() AS Integer
  io::print(os::version())
  io::print(os::name())
  io::print(os::arch())
  io::print(toString(os::uptime()))
  io::print(toString(os::isAdmin()))
  io::print(toString(os::pid()))
  io::print(os::hostName())
  io::print(os::userName())
  io::print(os::executablePath())
  io::print(toString(os::cpuCount()))
  io::print(os::getEnv(\"PATH\"))
  io::print(os::getEnvOr(\"NOPE\", \"fallback\"))
  io::print(toString(os::hasEnv(\"PATH\")))
  os::setEnv(\"MFB_COVERAGE\", \"1\")
  os::unsetEnv(\"MFB_COVERAGE\")
  LET env AS Map OF String TO String = os::environ()
  io::print(toString(len(env)))
  LET argv AS List OF String = os::args()
  io::print(toString(len(argv)))
  os::sleep(0)
  RETURN 0
END FUNC
";

fn program(target: CodeTarget) -> &'static NativeCodePlan {
    code_for_src_cached(SRC, target, Console)
}

fn body(target: CodeTarget, member: &str) -> &'static CodeFunction {
    code_function(program(target), &format!("runtime.os.{member}"))
}

/// The external symbols this body calls.
fn calls(f: &CodeFunction) -> Vec<String> {
    Stream::of(f).calls_between(0, f.instructions.len())
}

/// Every backend implements every `os::` member with a real body.
///
/// A registry descriptor does not force an implementation, so a family arm that
/// was never written lowers to something that returns whatever happens to be in
/// the result register. Requiring a call out to the platform (or, for the two
/// constants, a materialized string) is what makes "unimplemented on this target"
/// impossible to ship quietly.
#[test]
fn every_backend_implements_every_os_member() {
    for target in CodeTarget::ALL {
        for member in ["version", "uptime", "isAdmin", "pid"] {
            let f = body(target, member);
            let called = calls(f);
            assert!(
                !called.is_empty(),
                "{}: os::{member} must reach the platform on {}; it calls nothing",
                f.name,
                target.name()
            );
        }
    }
}

/// The privilege test's SENSE is inverted between Windows and everywhere else.
///
/// `IsUserAnAdmin()` returns non-zero for an administrator; `geteuid()` returns
/// **zero** for root. Copying one arm to the other keeps the code compiling and
/// the shape identical, and reports every ordinary user as an administrator (or
/// root as an ordinary user) — a privilege answer no test that runs as one user
/// can catch.
#[test]
fn is_admin_tests_the_opposite_sense_on_windows_and_unix() {
    for target in CodeTarget::ALL {
        let f = body(target, "isAdmin");
        let s = Stream::of(f);
        let probe = if target == CodeTarget::WindowsX86_64 {
            "IsUserAnAdmin"
        } else {
            "geteuid"
        };
        let called = calls(f);
        let asks = called.iter().filter(|t| t.contains(probe)).count();
        assert_eq!(
            asks,
            1,
            "{}: os::isAdmin must ask `{probe}` exactly once; it calls {called:?}",
            target.name()
        );
        let true_arm = s.label_starting("_mfb_rt_os_os_isAdmin_true");
        let (want, spelling) = if target == CodeTarget::WindowsX86_64 {
            // Non-zero from IsUserAnAdmin means "is an administrator".
            ("ne", "IsUserAnAdmin() != 0")
        } else {
            // euid 0 means root.
            ("eq", "geteuid() == 0")
        };
        assert_eq!(
            s.branch_condition_to(&true_arm),
            want,
            "{}: os::isAdmin must reach its TRUE arm on `{spelling}`; a copied arm \
             reports every ordinary user as an administrator",
            target.name()
        );
    }
}

/// Each backend reads the version from its own source, at its own offset.
///
/// These are silent-wrong-answer bugs: `uname`'s `release` is the THIRD 65-byte
/// field of `struct utsname` (offset 130), and `RTL_OSVERSIONINFOW`'s
/// `dwBuildNumber` sits at +12. A wrong constant still produces a plausible
/// string from a neighbouring field.
#[test]
fn version_reads_each_platforms_own_source() {
    for target in CodeTarget::ALL {
        let f = body(target, "version");
        let called = calls(f);
        let probe = match target {
            CodeTarget::MacosAarch64 => "sysctlbyname",
            CodeTarget::WindowsX86_64 => "RtlGetVersion",
            _ => "uname",
        };
        let asks = called.iter().filter(|t| t.contains(probe)).count();
        assert_eq!(
            asks,
            1,
            "{}: os::version must ask `{probe}` exactly once; it calls {called:?}",
            target.name()
        );
    }
    // Linux: `release` is utsname's third 65-byte field, so it sits exactly 130
    // bytes into the buffer `uname` was handed. Stated as the DIFFERENCE between
    // two stack addresses because `finalize_frame` shifts every sp-relative
    // access up past the callee-saved area, by a different amount per backend --
    // the emitter writes 0 and 130, and the stream carries 32/162 on x86-64 and
    // 16/146 on riscv64.
    for target in [
        CodeTarget::LinuxAarch64,
        CodeTarget::LinuxX86_64,
        CodeTarget::LinuxRiscv64,
    ] {
        let f = body(target, "version");
        let offsets = Stream::of(f).stack_offsets();
        let buffer = offsets.first().copied().unwrap_or_default();
        let release = offsets.iter().filter(|o| **o - buffer == 130).count();
        assert_eq!(
            release,
            1,
            "{}: os::version must read utsname.release 130 bytes into the buffer \
             it handed `uname` (the third 65-byte field); a wrong offset returns a \
             neighbouring field's text. Buffer at {buffer}, addresses taken: \
             {offsets:?}",
            target.name()
        );
    }
}

/// `os::name` and `os::arch` are compiled-in constants, not runtime queries.
///
/// They are the target's identity, known at compile time, so asking the OS for
/// them would make a cross-compiled binary report the machine that built it.
#[test]
fn name_and_arch_are_materialized_constants_not_queries() {
    for target in CodeTarget::ALL {
        for member in ["name", "arch"] {
            let f = body(target, member);
            let called = calls(f);
            // `!a && !b` leaves `b` unevaluated whenever `a` holds, and an
            // unevaluated operand is an uncovered region in THIS file.
            let internal = ["arena", "error"];
            let non_arena: Vec<&String> = called
                .iter()
                .filter(|t| !internal.iter().any(|part| t.contains(part)))
                .collect();
            assert!(
                non_arena.is_empty(),
                "{}: os::{member} is a compile-time constant and must query \
                 nothing; it calls {non_arena:?}",
                target.name()
            );
            let materialized = f
                .instructions
                .iter()
                .filter(|i| i.op == CodeOp::StrU8)
                .count();
            assert!(
                materialized > 0,
                "{}: os::{member} must materialize its bytes into a fresh String",
                target.name()
            );
        }
    }
    // ...and they disagree across targets, which is the whole point.
    let arch_of = |target: CodeTarget| {
        Stream::of(body(target, "arch"))
            .instructions
            .iter()
            .filter(|i| i.op == CodeOp::MovImm && Stream::field(i, "type") == "Byte")
            .filter_map(|i| Stream::field(i, "value").parse::<u8>().ok())
            .take_while(|b| *b != 0)
            .map(|b| b as char)
            .collect::<String>()
    };
    assert_eq!(arch_of(CodeTarget::LinuxX86_64), "x86_64");
    assert_eq!(arch_of(CodeTarget::LinuxAarch64), "aarch64");
    assert_eq!(arch_of(CodeTarget::LinuxRiscv64), "riscv64");
}

/// Every backend's `os::uptime` returns SECONDS.
///
/// Two of the three arms get a different unit from the platform and have to
/// convert: Windows' `GetTickCount64` is milliseconds, and macOS' boot time is an
/// absolute instant that must be subtracted from now. Only Linux's `sysinfo`
/// hands back seconds directly. A missing conversion is off by 1000x or by
/// decades, and reads as a plausible number in both cases.
#[test]
fn uptime_is_seconds_on_every_backend() {
    for target in CodeTarget::ALL {
        let f = body(target, "uptime");
        let called = calls(f);
        match target {
            CodeTarget::WindowsX86_64 => {
                let ticks = called
                    .iter()
                    .filter(|t| t.contains("GetTickCount64"))
                    .count();
                assert_eq!(
                    ticks, 1,
                    "windows os::uptime must read GetTickCount64; calls {called:?}"
                );
                let divides = f
                    .instructions
                    .iter()
                    .filter(|i| i.op == CodeOp::CmpImm && Stream::field(i, "rhs") == "1000")
                    .count();
                assert!(
                    divides > 0,
                    "windows os::uptime must convert milliseconds to seconds"
                );
            }
            CodeTarget::MacosAarch64 => {
                let sysctl = called.iter().filter(|t| t.contains("sysctl")).count();
                assert_eq!(
                    sysctl, 1,
                    "macos os::uptime must read KERN_BOOTTIME; calls {called:?}"
                );
                let subtracts = f
                    .instructions
                    .iter()
                    .filter(|i| i.op == CodeOp::Sub)
                    .count();
                assert!(
                    subtracts > 0,
                    "macos os::uptime must subtract the boot instant from now, \
                     not report the boot instant"
                );
            }
            _ => {
                let asks = called.iter().filter(|t| t.contains("sysinfo")).count();
                assert_eq!(
                    asks, 1,
                    "linux os::uptime must read sysinfo; calls {called:?}"
                );
                // Arch-neutral: the instruction immediately before the call must
                // form the buffer address from the stack pointer. Naming a
                // physical argument register would only be true on one backend.
                let s = Stream::of(f);
                let at = s.index_of("the sysinfo call", |i| {
                    i.get("target").is_some_and(|t| t.contains("sysinfo"))
                });
                let stage = &f.instructions[at - 1];
                assert_eq!(
                    stage.op,
                    CodeOp::AddImm,
                    "{}: os::uptime must stage the sysinfo buffer address from the \
                     stack pointer immediately before the call; without it sysinfo \
                     scribbles a 112-byte struct over whatever the caller left in \
                     the argument register",
                    target.name()
                );
                let from = Stream::field(stage, "src");
                let stack = ["sp", "rsp"].contains(&from.as_str());
                assert!(
                    stack,
                    "{}: the sysinfo buffer must come from this frame, not from `{from}`",
                    target.name()
                );
            }
        }
    }
}

/// The `abi_inline` signature every `os::` body under test has.
type OsBody = fn(&mut CodeBuilder, &[ValueResult], &AbiCtx) -> Result<ValueResult, String>;

/// Every `os::` body refuses to emit a call the platform import list does not
/// declare -- on every family.
///
/// These bodies reach libc on all three families, and `emit_external_call` fails
/// closed when the plan has not declared the symbol. That refusal is the only
/// thing between a mis-specified plan and an executable carrying a call to a
/// symbol nothing imports. No whole-program lowering can reach it: the plan
/// derives its import list from the very calls these bodies emit, so in a real
/// build the symbol is always there. Driving the emitters through a
/// `BuilderHarness` whose import list is EMPTY is the only way to see the arm --
/// and it has to be the real per-family platform, because `TestPlatform` lowers
/// every external call to a plain `bl` and never consults the list at all.
#[test]
fn every_os_body_refuses_an_undeclared_import() {
    use crate::target::linux_common::code::Platform as LinuxPlatform;
    let linux = LinuxPlatform::for_test(
        crate::target::linux_x86_64::code::X86_64,
        LinuxFlavor::Glibc,
    );
    let macos = crate::target::macos_aarch64::code::Platform;
    let windows = crate::target::win_x86_64::code::Platform;
    let platforms: [(&str, &dyn CodegenPlatform); 3] =
        [("linux", &linux), ("macos", &macos), ("windows", &windows)];
    let bodies: [(&str, OsBody); 3] = [
        ("version", os::func_version::lower_version),
        ("uptime", os::func_uptime::lower_uptime),
        ("isAdmin", os::func_is_admin::lower_is_admin),
    ];
    for (family, platform) in platforms {
        for (member, lower) in bodies {
            let harness = BuilderHarness::default();
            let mut builder = harness.builder(&format!("_mfb_rt_os_{member}"), platform);
            let ctx = harness.abi_ctx(platform);
            let err = lower(&mut builder, &[], &ctx).err().unwrap_or_default();
            assert!(
                err.contains("import"),
                "{family}: os::{member} must refuse to lower when the platform \
                 declares no libc import for it, rather than emitting a call \
                 nothing resolves; got {err:?}"
            );
        }
    }
}

/// `register` publishes exactly the `os::` surface, and nothing silently drops
/// out of it.
///
/// The list is spelled out rather than derived from the registry, because
/// deriving it would make the test agree with whatever `register` happens to do:
/// a member deleted from the body would delete itself from the expectation too.
/// The one thing a spelled-out list costs -- an edit when the package genuinely
/// grows -- is the point.
///
/// It also keeps `register` measurable. Every `func_*::register(&mut pkg)` line
/// is inlined away when nothing in the module is `#[cfg(test)]`, so those 23
/// lines are simply absent from the coverage report; the moment this file
/// exists they become countable, and without a caller they would be countable
/// and dead -- this suite would drop `os/mod.rs` from 100% to 85% purely by
/// existing.
#[test]
fn register_publishes_the_whole_os_surface() {
    let mut registry = crate::codegen::registry::Registry::new();
    os::register(&mut registry);
    let package = registry
        .resolve_package("os")
        .map(|p| p.functions().iter().map(|f| f.name).collect::<Vec<_>>())
        .unwrap_or_default();
    let expected = [
        "getEnv",
        "getEnvOr",
        "hasEnv",
        "setEnv",
        "unsetEnv",
        "environ",
        "args",
        "pid",
        "executablePath",
        "resourcePath",
        "name",
        "arch",
        "hostName",
        "userName",
        "cpuCount",
        "version",
        "uptime",
        "isAdmin",
        "sleep",
    ];
    for member in expected {
        assert!(
            package.contains(&member),
            "os::{member} must be registered; the package publishes {package:?}"
        );
    }
    assert_eq!(
        package.len(),
        expected.len(),
        "the os package publishes {} members but this test knows {}; a member was \
         added or removed without updating the list. Published: {package:?}",
        package.len(),
        expected.len()
    );
}

/// `os::resourcePath` lowers on EVERY backend, Windows included.
///
/// This test used to assert the opposite, and it was right when it was written:
/// the member reads the executable's own directory through a raw-buffer helper
/// that had no Windows implementation, so `os.resourcePath` was absent from
/// `win_x86_64`'s `SUPPORTED_RUNTIME_CALLS` and a cross-build for Windows was
/// refused. That was bug-454, and main fixed it (94b2ec1e1) -- Windows now
/// shares `os.executablePath`'s `GetModuleFileNameW` acquisition.
///
/// Rewritten rather than deleted, because the property is worth more now than
/// the refusal was. `os::resourcePath` is how a program finds the assets the
/// build copied beside it, and a project that uses it must be buildable for
/// every target the compiler claims to support -- a member that lowers on four
/// of five makes the fifth a build failure discovered at release time.
#[test]
fn resource_path_lowers_on_every_backend() {
    const SRC: &str = "\
IMPORT io
IMPORT os

FUNC main() AS Integer
  io::print(os::resourcePath(\"data.txt\"))
  RETURN 0
END FUNC
";
    for target in CodeTarget::ALL {
        assert!(
            try_code_for_src(SRC, target, Console).is_ok(),
            "os::resourcePath must lower on {} -- a member that lowers on four \
             of the five targets makes the fifth a build failure nobody sees \
             until a release runner reaches it",
            target.name()
        );
    }
}
