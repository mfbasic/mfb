//! Every POSIX threading primitive the shared thread code names has a realization on
//! every backend, and Windows' is the right Win32 call.
//!
//! The thread runtime is written once against the pthread vocabulary (`pthread_mutex_lock`,
//! `pthread_cond_timedwait`, `nanosleep`, `sched_yield`, ...). On macOS and Linux each name
//! is a call to the libc symbol; Windows has no pthreads, so `emit_thread_external_call`
//! translates each to its Win32 equivalent (SRWLOCK, CONDITION_VARIABLE, `Sleep`,
//! `SwitchToThread`, `GetSystemTimePreciseAsFileTime`) and leaves the POSIX result (`0` =
//! success) in the C return register, or emits nothing for the stack-size attribute calls
//! that are inert there. Which primitives a program reaches depends on what it does with
//! threads, so a primitive none of the corpus's programs uses had never been translated;
//! this drives each name directly, on each backend, with the imports a threaded program's
//! plan declares, and asserts the call each lands on. A name outside the vocabulary is
//! refused on Windows by name, never silently dropped.

use std::collections::HashMap;

use super::platforms::platform_for;
use crate::codegen::engine::builder::EmitCtx;
use crate::codegen::engine::mir;
use crate::codegen::runtime::thread::emit_thread_external_call;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, CodeTarget};

const THREADED: &str = "\
IMPORT thread

ISOLATED FUNC work(w AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC main() AS Integer
  RES t AS Thread OF Integer TO Integer = thread::start(work, 1)
  RETURN thread::waitFor(t)
END FUNC
";

/// `(POSIX name, the Win32 symbol it must reach, or `None` for an inert one)`.
const WINDOWS: &[(&str, Option<&str>)] = &[
    ("pthread_mutex_init", Some("InitializeSRWLock")),
    ("pthread_mutex_lock", Some("AcquireSRWLockExclusive")),
    ("pthread_mutex_unlock", Some("ReleaseSRWLockExclusive")),
    ("pthread_cond_init", Some("InitializeConditionVariable")),
    ("pthread_cond_signal", Some("WakeConditionVariable")),
    ("pthread_cond_broadcast", Some("WakeAllConditionVariable")),
    ("pthread_cond_wait", Some("SleepConditionVariableSRW")),
    ("pthread_cond_timedwait", Some("SleepConditionVariableSRW")),
    ("nanosleep", Some("Sleep")),
    ("pthread_detach", Some("CloseHandle")),
    ("pthread_join", Some("WaitForSingleObject")),
    ("pthread_attr_init", None),
    ("pthread_attr_setstacksize", None),
    ("pthread_attr_destroy", None),
    ("sched_yield", Some("SwitchToThread")),
    ("clock_gettime", Some("GetSystemTimePreciseAsFileTime")),
];

fn emit(
    target: CodeTarget,
    name: &str,
) -> Result<Vec<crate::codegen::engine::types::CodeRelocation>, String> {
    let plan = code_for_src_cached(THREADED, target, Console);
    let imports: HashMap<String, String> = plan
        .imports
        .iter()
        .map(|import| (import.symbol.clone(), import.library.clone()))
        .collect();
    let platform = platform_for(target);
    mir::set_backend(platform.backend());
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: "_mfb_probe",
            platform_imports: &imports,
            platform: &*platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        name,
    )?;
    Ok(relocations)
}

#[test]
fn windows_translates_each_pthread_primitive_to_its_win32_call() {
    for (name, win32) in WINDOWS {
        let relocations = emit(CodeTarget::WindowsX86_64, name)
            .unwrap_or_else(|err| panic!("windows must translate {name}: {err}"));
        let reached: Vec<&str> = relocations.iter().map(|r| r.to.as_str()).collect();
        match win32 {
            Some(symbol) => assert!(
                reached.contains(symbol),
                "{name} must reach {symbol}; it reached {reached:?}"
            ),
            None => assert!(
                reached.is_empty(),
                "{name} is inert on Windows (the stack size is set on CreateThread) but \
                 called {reached:?}"
            ),
        }
    }
}

#[test]
fn windows_refuses_a_primitive_it_has_no_translation_for() {
    let err = match emit(CodeTarget::WindowsX86_64, "pthread_barrier_wait") {
        Err(err) => err,
        Ok(_) => panic!("an unknown primitive must not be dropped"),
    };
    assert!(
        err.contains("no Win32 translation for thread primitive 'pthread_barrier_wait'"),
        "{err}"
    );
}

#[test]
fn the_posix_backends_call_the_libc_symbol_by_its_own_name() {
    for target in [
        CodeTarget::LinuxX86_64,
        CodeTarget::LinuxAarch64,
        CodeTarget::LinuxRiscv64,
        CodeTarget::MacosAarch64,
    ] {
        for name in ["pthread_mutex_lock", "pthread_cond_wait", "pthread_join"] {
            let relocations = emit(target, name)
                .unwrap_or_else(|err| panic!("{} must call {name}: {err}", target.name()));
            let reached: Vec<&str> = relocations.iter().map(|r| r.to.as_str()).collect();
            let expected = if target == CodeTarget::MacosAarch64 {
                format!("_{name}")
            } else {
                name.to_string()
            };
            assert_eq!(
                reached,
                [expected.as_str()],
                "{}: {name} is a call to the libc symbol",
                target.name()
            );
        }
    }
}

/// A program that draws random numbers AND starts a thread.
const THREADED_RNG: &str = "\
IMPORT thread
IMPORT math

ISOLATED FUNC work(w AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer
  RETURN math::rand(0, n)
END FUNC

FUNC main() AS Integer
  LET seeded = math::rand(0, 9)
  RES t AS Thread OF Integer TO Integer = thread::start(work, 5)
  RETURN thread::waitFor(t) + seeded
END FUNC
";

/// A new thread gets its own random stream, seeded from the SPAWNING thread's generator,
/// and only when the program uses randomness at all.
#[test]
fn a_thread_started_by_a_program_that_uses_rand_is_seeded_from_its_parent() {
    use crate::codegen::error::constants::{RNG_NEXT_SYMBOL, RNG_SEED_SYMBOL};
    let reaches = |source: &str, symbol: &str| {
        code_for_src_cached(source, CodeTarget::LinuxX86_64, Console)
            .functions
            .iter()
            .any(|function| function.relocations.iter().any(|r| r.to == symbol))
    };
    for symbol in [RNG_NEXT_SYMBOL, RNG_SEED_SYMBOL] {
        assert!(
            reaches(THREADED_RNG, symbol),
            "spawn must draw the child's seed from the parent's generator ({symbol})"
        );
        assert!(
            !reaches(THREADED, symbol),
            "a program that never draws a random number seeds no stream ({symbol})"
        );
    }
    for target in CodeTarget::ALL {
        let result = crate::testutil::try_code_for_src(THREADED_RNG, target, Console);
        assert!(result.is_ok(), "{}: {:?}", target.name(), result.err());
    }
}
