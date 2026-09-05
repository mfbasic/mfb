//! A program that starts threads, lowered for every backend.
//!
//! Only 4 of the 47 committed thread fixtures lower in this harness — the rest
//! reach their worker through an imported package — so the thread lowering had
//! almost no in-process coverage. This is one program with three ISOLATED
//! workers returning a scalar, an owning String, and a collection of owning
//! Strings, which is the shape spread that matters: an arena is PER-THREAD
//! (`.ai/canvas-threading.md` §2), so a scalar can come back in a register and
//! the other two cannot.
//!
//! The assertions are deliberately about what the emitted plan SAYS, not about
//! how the result copy is implemented. A first draft asserted that each
//! transferred type gets an `_mfb_thread_copy_<type>_<hash>` function and that
//! `runtime.thread.waitFor` allocates; neither is true for this program — the
//! copy functions are emitted for recursive types, and `waitFor` only waits —
//! and a test that pins a mechanism it has not verified is worse than no test,
//! because the next reader believes it.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::NativeCodePlan;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, CodeTarget};

/// Workers returning each shape the transfer has a separate arm for: a scalar,
/// an owning value, and a collection of owning values.
const SRC: &str = "\
IMPORT collections
IMPORT io
IMPORT thread

ISOLATED FUNC counts(t AS ThreadWorker OF Nothing TO Integer, seed AS Integer) AS Integer
  RETURN seed + 1
END FUNC

ISOLATED FUNC names(t AS ThreadWorker OF Nothing TO String, seed AS Integer) AS String
  RETURN \"worker-\" & toString(seed)
END FUNC

ISOLATED FUNC listing(t AS ThreadWorker OF Nothing TO List OF String, seed AS Integer) AS List OF String
  MUT out AS List OF String = []
  FOR i = 0 TO seed
    out = collections::append(out, \"item\" & toString(i))
  NEXT
  RETURN out
END FUNC

FUNC main() AS Integer
  LET a AS Thread OF Nothing TO Integer = thread::start(counts, 10)
  LET b AS Thread OF Nothing TO String = thread::start(names, 2)
  LET c AS Thread OF Nothing TO List OF String = thread::start(listing, 3)
  LET n AS Integer = thread::waitFor(a)
  LET s AS String = thread::waitFor(b)
  LET l AS List OF String = thread::waitFor(c)
  io::print(s)
  io::print(toString(n + len(l)))
  RETURN 0
END FUNC
";

fn program(target: CodeTarget) -> &'static NativeCodePlan {
    code_for_src_cached(SRC, target, Console)
}

/// Every ISOLATED worker is emitted once, with a body, on every backend.
///
/// `thread::start(worker, seed)` names the entry the spawned thread runs. A
/// worker that was not emitted leaves the trampoline branching at a symbol
/// nothing defines; a worker emitted with an empty body starts a thread that
/// returns immediately, and `waitFor` then hands back whatever the result slot
/// was initialized to -- a plausible value, on time, and wrong.
#[test]
fn every_isolated_worker_is_emitted_once_with_a_body() {
    for target in CodeTarget::ALL {
        let plan = program(target);
        for worker in ["counts", "names", "listing"] {
            let bodies: Vec<&crate::codegen::engine::types::CodeFunction> =
                plan.functions.iter().filter(|f| f.name == worker).collect();
            assert_eq!(
                bodies.len(),
                1,
                "{}: `{worker}` must be emitted exactly once; it was emitted {} \
                 time(s)",
                target.name(),
                bodies.len()
            );
            assert!(
                bodies[0].instructions.len() > 1,
                "{}: `{worker}` must have a body -- an empty one returns \
                 immediately and `waitFor` hands back the result slot's initial \
                 value",
                target.name()
            );
        }
    }
}

/// The three workers are distinct symbols.
///
/// They differ only in their return type, and two of them (`names`, `listing`)
/// return owning values. A symbol collision would silently run one worker's body
/// for another's `thread::start`, producing a value of the wrong SHAPE that the
/// joining side then reads through the expected type's layout.
#[test]
fn the_workers_do_not_share_a_symbol() {
    for target in CodeTarget::ALL {
        let plan = program(target);
        let mut symbols: Vec<&str> = plan
            .functions
            .iter()
            .filter(|f| matches!(f.name.as_str(), "counts" | "names" | "listing"))
            .map(|f| f.symbol.as_str())
            .collect();
        let total = symbols.len();
        symbols.sort_unstable();
        symbols.dedup();
        assert_eq!(
            symbols.len(),
            total,
            "{}: two workers share a symbol, so one `thread::start` would run the \
             other's body: {symbols:?}",
            target.name()
        );
    }
}

/// Every backend defines the thread symbols it calls.
///
/// The worker does not enter the language entry directly: it enters a
/// trampoline that establishes arena state first, because arena state is a
/// callee-saved register a spawned thread starts without. A backend that named
/// a trampoline it did not define, or that reached one through an import it did
/// not declare, would not link.
#[test]
fn every_backend_defines_the_thread_symbols_it_calls() {
    for target in CodeTarget::ALL {
        let plan = program(target);
        let defined: Vec<&str> = plan.functions.iter().map(|f| f.symbol.as_str()).collect();
        let called: Vec<String> = plan
            .functions
            .iter()
            .flat_map(|f| {
                f.instructions
                    .iter()
                    .filter(|i| i.op == CodeOp::BranchLink)
                    .filter_map(|i| i.get("target"))
            })
            .filter(|t| t.contains("thread"))
            .collect();
        for symbol in &called {
            if plan.imports.iter().any(|i| &i.symbol == symbol) {
                continue;
            }
            assert!(
                defined.contains(&symbol.as_str()),
                "{}: the plan calls `{symbol}` and neither defines nor imports it",
                target.name()
            );
        }
        assert!(
            !called.is_empty(),
            "{}: a program that starts three threads must call into the thread \
             runtime",
            target.name()
        );
    }
}
