//! bug-843 / bug-842: a closure carries the resources it captures, and every
//! movement of the closure moves them (`resource/closure_carry.rs`, with
//! `resource/{walk,owner,module_owner}.rs` for the type walk, the owner tokens and
//! the module instance's list).
//!
//! What a runtime test cannot see is the SHAPE the compiler gives that rule, and the
//! shape is the contract: a closure's environment routine (`runtime.closureEnv`) is
//! where the resources it captured are closed, released, claimed, adopted and
//! floated to the module, and a movement of a closure between scopes is a call to
//! that routine (an `<indirect>` call: the routine's address lives in the closure).
//! `tests/rt-behavior/memory/closure-capture-resource-append-rt` proves the
//! behaviour end to end; these lower the same shapes in process, on every backend.
//!
//! Each source is a family of cases the type walk and the owner tokens treat
//! differently: a captured resource UNION (closed by dispatching on its tag), a
//! record with a recursive field that reaches a resource (walked through a deferred
//! worklist), a captured `STATE` resource, a closure that captures a closure, and
//! the movements of a closure value — returned beside a resource temporary,
//! discarded, stored into a module-level binding, appended to a module-level list.

use crate::codegen::engine::tests::test_support::Stream;
use crate::codegen::engine::types::{CodeFunction, NativeCodePlan};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, fixture_src, CodeTarget};

const FS_CLOSE: &str = "_mfb_rt_fs_fs_close";
const TCP_CLOSE: &str = "_mfb_rt_tcp_tcp_close";
const INDIRECT: &str = "<indirect>";

/// A resource union and a record reaching a resource through a recursive field, each
/// captured by a closure.
const UNION_AND_TREE: &str = r#"IMPORT io
IMPORT fs
IMPORT tcp
IMPORT collections

UNION Handle
  fs::File
  tcp::Socket
END UNION

TYPE Node
  f AS RES fs::File
  kids AS List OF Node
END TYPE

FUNC mkHandle() AS RES Handle
  RES h AS Handle = fs::createTempFile()
  RETURN h
END FUNC

FUNC kind(h AS Handle) AS Integer
  MATCH h
    CASE fs::File(f)
      RETURN 1
    CASE tcp::Socket(s)
      RETURN 2
  END MATCH
END FUNC

FUNC mkNode() AS Node
  RES f AS fs::File = fs::createTempFile()
  RETURN Node[f, []]
END FUNC

FUNC unionCarrier() AS FUNC() AS Integer
  RES h AS Handle = mkHandle()
  RETURN LAMBDA() -> kind(h)
END FUNC

FUNC treeKeeper() AS Integer
  LET n = mkNode()
  LET g = LAMBDA() -> len(n.kids)
  RETURN g()
END FUNC

FUNC main() AS Integer
  LET g = unionCarrier()
  LET k = treeKeeper()
  io::print(toString(g() + k))
  RETURN 0
END FUNC
"#;

/// The movements of a closure carrying a resource, through a call argument, a
/// discarded statement, and module-level bindings of every shape a store can take.
const MOVEMENTS: &str = r#"IMPORT io
IMPORT fs
IMPORT tcp
IMPORT collections

UNION Handle
  fs::File
  tcp::Socket
END UNION

TYPE Holder
  label AS String
  f AS RES fs::File
END TYPE

MUT gBoxes AS List OF Holder = []
MUT gFiles AS List OF RES fs::File = []
MUT gMap AS Map OF String TO RES fs::File = Map OF String TO RES fs::File {}
MUT gUnion AS FUNC() AS Integer = LAMBDA() -> 0
MUT gHandles AS List OF RES Handle = []

FUNC mkHandle() AS RES Handle
  RES h AS Handle = fs::createTempFile()
  RETURN h
END FUNC

FUNC kind(h AS Handle) AS Integer
  MATCH h
    CASE fs::File(f)
      RETURN 1
    CASE tcp::Socket(s)
      RETURN 2
  END MATCH
END FUNC

FUNC wrapHandle(h AS Handle) AS FUNC() AS Integer
  RETURN LAMBDA() -> kind(h)
END FUNC

FUNC wrapFile(f AS fs::File) AS FUNC() AS Integer
  RETURN LAMBDA() -> fs::size(f)
END FUNC

FUNC returnedViaTemp() AS FUNC() AS Integer
  RETURN wrapFile(fs::createTempFile())
END FUNC

FUNC returnedUnionViaTemp() AS FUNC() AS Integer
  RETURN wrapHandle(mkHandle())
END FUNC

FUNC discards() AS Integer
  wrapHandle(mkHandle())
  mkHandle()
  LET k = kind(mkHandle())
  RETURN k
END FUNC

FUNC keepUnion() AS Nothing
  RES h AS Handle = mkHandle()
  gUnion = LAMBDA() -> kind(h)
END FUNC

FUNC keepHandles() AS Nothing
  RES h AS Handle = mkHandle()
  gHandles = collections::append(gHandles, h)
END FUNC

FUNC keepMap() AS Nothing
  RES f AS fs::File = fs::createTempFile()
  gMap = Map OF String TO RES fs::File { "a" := f }
END FUNC

FUNC keepWith(h AS Holder) AS Nothing
  gBoxes = collections::append(gBoxes, WITH h { label := "x" })
END FUNC

FUNC keepMember(h AS Holder) AS Nothing
  gFiles = collections::append(gFiles, h.f)
END FUNC

FUNC main() AS Integer
  LET a = returnedViaTemp()
  LET b = returnedUnionViaTemp()
  LET c = discards()
  keepUnion()
  keepHandles()
  keepMap()
  RES f AS fs::File = fs::createTempFile()
  LET h = Holder["x", f]
  keepWith(h)
  keepMember(h)
  io::print(toString(a() + b() + c + gUnion()))
  RETURN 0
END FUNC
"#;

/// `STATE` resources, a resource union with `STATE`, and closures that capture
/// closures that capture resources.
const STATE_AND_NESTED: &str = r#"IMPORT io
IMPORT fs
IMPORT tcp
IMPORT collections

UNION Handle
  fs::File
  tcp::Socket
END UNION

TYPE Cur
  n AS Integer
END TYPE

TYPE Bag
  run AS FUNC() AS Integer
  tag AS String
END TYPE

MUT gNested AS FUNC() AS Integer = LAMBDA() -> 0
MUT gBag AS List OF Bag = []

FUNC mkState() AS RES fs::File STATE Cur
  RES f AS fs::File STATE Cur = fs::createTempFile()
  f.state.n = 5
  RETURN f
END FUNC

FUNC mkUnionState() AS RES Handle STATE Cur
  RES h AS Handle STATE Cur = fs::createTempFile()
  h.state.n = 7
  RETURN h
END FUNC

FUNC plain() AS RES fs::File
  RES f AS fs::File = fs::createTempFile()
  RETURN f
END FUNC

FUNC stateCarrier() AS FUNC() AS Integer
  RES f AS fs::File STATE Cur = mkState()
  RES p AS fs::File = plain()
  RETURN LAMBDA() -> f.state.n + fs::size(p)
END FUNC

FUNC unionStateCarrier() AS FUNC() AS Integer
  RES h AS Handle STATE Cur = mkUnionState()
  RETURN LAMBDA() -> h.state.n
END FUNC

FUNC nestedCarrier() AS FUNC() AS Integer
  LET inner = stateCarrier()
  LET other = unionStateCarrier()
  RETURN LAMBDA() -> inner() + other()
END FUNC

FUNC callIt(g AS FUNC() AS Integer) AS Integer
  RETURN g()
END FUNC

FUNC bagUp() AS Bag
  RETURN Bag[nestedCarrier(), "t"]
END FUNC

FUNC keepNested() AS Nothing
  gNested = nestedCarrier()
  gBag = collections::append(gBag, Bag[nestedCarrier(), "g"])
END FUNC

FUNC main() AS Integer
  LET a = callIt(nestedCarrier())
  LET b = bagUp()
  nestedCarrier()
  stateCarrier()
  unionStateCarrier()
  keepNested()
  MUT gs AS List OF FUNC() AS Integer = []
  IF a > 0 THEN
    gs = collections::append(gs, unionStateCarrier())
    gs = collections::append(gs, nestedCarrier())
  END IF
  LET r = b.run
  io::print(toString(a + r() + gNested() + len(gs)))
  RETURN 0
END FUNC
"#;

/// A program whose closures capture only an `Integer`: nothing to carry.
const NOTHING_CAPTURED: &str = "\
IMPORT io

FUNC adder(n AS Integer) AS FUNC(Integer) AS Integer
  RETURN LAMBDA(x AS Integer) -> x + n
END FUNC

FUNC main() AS Integer
  LET f = adder(2)
  io::print(toString(f(3)))
  RETURN 0
END FUNC
";

fn plan(source: &str) -> &'static NativeCodePlan {
    code_for_src_cached(source, CodeTarget::LinuxX86_64, Console)
}

fn function<'a>(plan: &'a NativeCodePlan, name: &str) -> &'a CodeFunction {
    crate::testutil::code_function(plan, name)
}

fn calls(function: &CodeFunction) -> Vec<String> {
    Stream::of(function).calls_between(0, function.instructions.len())
}

fn count(calls: &[String], target: &str) -> usize {
    calls.iter().filter(|call| call.as_str() == target).count()
}

/// Every environment routine of the lowered module.
fn environments(plan: &NativeCodePlan) -> Vec<&CodeFunction> {
    plan.functions
        .iter()
        .filter(|function| function.name == "runtime.closureEnv")
        .collect()
}

/// Every program lowers on every backend. The carry operations are shared
/// codegen emitted per ISA; a backend that could not emit an environment routine
/// would be a closure that leaks or double-closes only on that machine.
#[test]
fn every_closure_carry_shape_lowers_on_every_backend() {
    let fixture = fixture_src("closure-capture-resource-append-rt");
    for (label, source) in [
        ("union and tree", UNION_AND_TREE),
        ("movements", MOVEMENTS),
        ("state and nested", STATE_AND_NESTED),
        ("bug-843 fixture", fixture.as_str()),
    ] {
        for target in CodeTarget::ALL {
            let result = crate::testutil::try_code_for_src(source, target, Console);
            assert!(
                result.is_ok(),
                "{label} must lower on {}: {:?}",
                target.name(),
                result.err()
            );
        }
    }
}

/// A closure that captures a resource closes it from its own environment routine:
/// one routine per capturing `LAMBDA`, each calling the resource's close op.
#[test]
fn a_capturing_closure_closes_its_resource_from_its_environment_routine() {
    let fixture = fixture_src("closure-capture-resource-append-rt");
    let plan = plan(&fixture);
    let envs = environments(plan);
    assert!(
        envs.len() >= 5,
        "the five capturing closures of the fixture each need an environment routine; \
         found {}",
        envs.len()
    );
    for env in envs {
        assert!(
            calls(env).iter().any(|call| call == FS_CLOSE),
            "{}: a closure over an `fs::File` must close it through the file's close op",
            env.symbol
        );
    }
}

/// A captured resource UNION is closed by dispatching on the active variant: the
/// environment routine reaches both variants' close ops.
#[test]
fn a_captured_resource_union_is_closed_by_variant() {
    let plan = plan(UNION_AND_TREE);
    let union_env = environments(plan)
        .into_iter()
        .map(calls)
        .find(|calls| calls.iter().any(|call| call == TCP_CLOSE))
        .expect("a closure over a File|Socket union must have a routine that can close a Socket");
    assert!(
        union_env.iter().any(|call| call == FS_CLOSE),
        "the same routine must close a File variant too; it called {union_env:?}"
    );
}

/// A closure that captures a closure forwards each carry operation to the inner
/// closure's own routine, and does not itself close anything.
#[test]
fn a_closure_over_a_closure_forwards_to_the_inner_routine() {
    let plan = plan(STATE_AND_NESTED);
    let forwarding: Vec<Vec<String>> = environments(plan)
        .into_iter()
        .map(calls)
        .filter(|calls| count(calls, INDIRECT) > 0)
        .collect();
    assert!(
        !forwarding.is_empty(),
        "an environment routine of a closure capturing closures must call the inner \
         closures' routines indirectly"
    );
    for calls in &forwarding {
        assert!(
            !calls
                .iter()
                .any(|call| call == FS_CLOSE || call == TCP_CLOSE),
            "a routine that only forwards must not close a resource itself: {calls:?}"
        );
    }
}

/// The movements of a closure between scopes are calls to its routine.
#[test]
fn moving_a_closure_calls_its_environment_routine() {
    let plan = plan(MOVEMENTS);
    // Returned beside a resource temporary: claimed (through the routine) before
    // the temporary closes, released after.
    let returned = calls(function(plan, "returnedViaTemp"));
    assert!(
        count(&returned, INDIRECT) >= 2,
        "a returned closure is claimed and then released through its routine: {returned:?}"
    );
    // A closure result nobody takes is closed at the statement.
    let discarded = calls(function(plan, "discards"));
    assert!(
        count(&discarded, INDIRECT) >= 1,
        "a discarded closure result closes what it carries through its routine: {discarded:?}"
    );
    // Stored into a module-level binding: the module instance takes the resources
    // through the routine's MODULE_FLOAT operation.
    let kept = calls(function(plan, "keepUnion"));
    assert!(
        count(&kept, INDIRECT) >= 1,
        "a closure stored into a global floats its resources through its routine: {kept:?}"
    );
    // A union temporary passed to a call is closed by tag at the statement.
    let temp = calls(function(plan, "discards"));
    assert!(
        temp.iter().any(|call| call == FS_CLOSE) && temp.iter().any(|call| call == TCP_CLOSE),
        "a resource-union temporary closes by variant: {temp:?}"
    );
}

/// The cost clause: a module none of whose closures captures a resource lowers its
/// environment routines (if any) without a close op.
#[test]
fn closures_that_capture_nothing_carry_no_resource_operations() {
    let plan = plan(NOTHING_CAPTURED);
    for env in environments(plan) {
        let calls = calls(env);
        assert!(
            !calls
                .iter()
                .any(|call| call == FS_CLOSE || call == TCP_CLOSE),
            "{}: a closure over an Integer has no resource to close: {calls:?}",
            env.symbol
        );
    }
}
