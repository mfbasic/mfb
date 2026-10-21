//! The committed resource programs lower, and the owner machinery they exercise
//! emits what each was written for.
//!
//! `resource/walk.rs` is the one type walk every resource consumer shares — scope
//! drops, returns, discards, module stores — and `resource/module_owner.rs` hands
//! a resource stored into a module-level binding to the module instance. A walk
//! over a `Map OF K TO RES V`, a store of a concrete resource into a union-typed
//! module slot, and the by-variant drain of a resource-union collection are each a
//! different arm of those two files, and each was written against one fixture
//! under `tests/rt-behavior/resources/`. The fixtures run for real in the
//! acceptance matrix; what only a lowering in process says is that the same
//! program still lowers on every backend and still reaches the resource's close
//! op from the arm the fixture is about.

use crate::codegen::engine::tests::test_support::Stream;
use crate::codegen::engine::types::NativeCodePlan;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_fixture, fixture_src, try_code_for_src, CodeTarget};

const FS_CLOSE: &str = "_mfb_rt_fs_fs_close";
const TCP_CLOSE: &str = "_mfb_rt_tcp_tcp_close";

/// Every fixture here owns resources through a collection, a union, a record
/// field or a module-level binding. All of them lower on every backend.
const FIXTURES: &[&str] = &[
    "append-res-list-literal-rt",
    "bug1027_member_into_module_union_rt",
    "bug427_list_union_state_rt",
    "bug429_owned_list_union_drain_rt",
    "bug643-collection-helpers-keep-caller-handles-rt",
    "float-through-repackaging-collections-rt",
    "inline-trap-collection-escape-rt",
    "inline-trap-producer-float-rt",
    "resource-collection-close-floated-valid",
    "resource-collection-floats-runtime",
    "resource-collection-map-value-runtime",
    "resource-collection-not-owner-valid",
    "resource-collection-transfer-runtime",
];

/// Stores into module-level bindings, one per operand shape the store analysis
/// distinguishes: a member of another global, the result of a call, a member of a
/// call result, and a binding that holds no resource at all.
const MODULE_STORES: &str = r#"IMPORT io
IMPORT fs
IMPORT collections

TYPE Holder
  label AS String
  f AS RES fs::File
END TYPE

RES gf AS fs::File = fs::createTempFile()
MUT gHolder AS Holder = Holder["g", gf]
MUT gFiles AS List OF RES fs::File = []
MUT counter AS Integer = 0
MUT gBoxes AS List OF Holder = []

FUNC mkFiles() AS List OF RES fs::File
  RES f AS fs::File = fs::createTempFile()
  RETURN [f]
END FUNC

FUNC mkHolder() AS Holder
  RES f AS fs::File = fs::createTempFile()
  RETURN Holder["m", f]
END FUNC

FUNC bump() AS Nothing
  counter = counter + 1
END FUNC

FUNC fromGlobalMember() AS Nothing
  gFiles = collections::append(gFiles, gHolder.f)
END FUNC

FUNC fromCall() AS Nothing
  gFiles = mkFiles()
END FUNC

FUNC fromCallMember() AS Nothing
  gFiles = collections::append(gFiles, mkHolder().f)
END FUNC

FUNC fromTrap() AS Nothing
  RES f AS fs::File = fs::createTempFile()
  gFiles = collections::insert(gFiles, 0, f) TRAP(e)
    RECOVER gFiles
  END TRAP
END FUNC

FUNC fromNamedParts(h AS Holder) AS Nothing
  RES f AS fs::File = fs::createTempFile()
  gBoxes = collections::append(gBoxes, Holder[h.label, f])
END FUNC

FUNC main() AS Integer
  bump()
  fromTrap()
  fromNamedParts(gHolder)
  fromGlobalMember()
  fromCall()
  fromCallMember()
  io::print(toString(counter + len(gFiles)))
  RETURN 0
END FUNC
"#;

/// Resource-union bindings that alias one another, wrap a concrete resource, are
/// returned, floated into a collection through an inline `TRAP` and returned with a
/// `STATE`.
const UNION_ALIASES: &str = r#"IMPORT io
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

FUNC mkHandle() AS RES Handle
  RES h AS Handle = fs::createTempFile()
  RETURN h
END FUNC

FUNC pickFail(n AS Integer) AS RES Handle
  IF n < 0 THEN
    FAIL error(77050001, "no handle")
  END IF
  RES h AS Handle = fs::createTempFile()
  RETURN h
END FUNC

FUNC aliasChain() AS RES Handle
  RES a AS Handle = mkHandle()
  RES b AS Handle = a
  RES c AS Handle = b
  RETURN c
END FUNC

FUNC wrapConcrete() AS RES Handle
  RES f AS fs::File = fs::createTempFile()
  RES c AS Handle = f
  RETURN c
END FUNC

FUNC wrapChain() AS RES Handle
  RES f AS fs::File = fs::createTempFile()
  RES c AS Handle = f
  RES d AS Handle = c
  RETURN d
END FUNC

FUNC parameterReturned(h AS Handle) AS Handle
  RETURN h
END FUNC

FUNC floatedUnion() AS Integer
  MUT hs AS List OF RES Handle = []
  MUT n AS Integer = 0
  WHILE n < 2
    RES h AS Handle = mkHandle()
    hs = collections::append(hs, h) TRAP(e)
      RECOVER hs
    END TRAP
    n = n + 1
  END WHILE
  RETURN len(hs)
END FUNC

FUNC floatedUnionFromFailing() AS Integer
  MUT hs AS List OF RES Handle = []
  MUT n AS Integer = 0
  WHILE n < 2
    hs = collections::append(hs, pickFail(n - 1)) TRAP(e)
      RECOVER hs
    END TRAP
    n = n + 1
  END WHILE
  RETURN len(hs)
END FUNC

FUNC stateReturned() AS RES fs::File STATE Cur
  RES f AS fs::File STATE Cur = fs::createTempFile()
  f.state.n = 3
  RETURN f
END FUNC

FUNC main() AS Integer
  RES a AS Handle = aliasChain()
  RES b AS Handle = wrapConcrete()
  RES c AS Handle = wrapChain()
  RES d AS Handle = parameterReturned(a)
  RES s AS fs::File STATE Cur = stateReturned()
  io::print(toString(floatedUnion() + floatedUnionFromFailing() + s.state.n))
  RETURN 0
END FUNC
"#;

fn closes(plan: &NativeCodePlan) -> Vec<String> {
    plan.functions
        .iter()
        .flat_map(|function| Stream::of(function).calls_between(0, function.instructions.len()))
        .filter(|call| call == FS_CLOSE || call == TCP_CLOSE)
        .collect()
}

/// Every resource-owning fixture lowers on every backend.
#[test]
fn every_resource_owning_fixture_lowers_on_every_backend() {
    for fixture in FIXTURES {
        let source = fixture_src(fixture);
        for target in CodeTarget::ALL {
            if let Err(err) = try_code_for_src(&source, target, Console) {
                panic!("{fixture} must lower on {}: {err}", target.name());
            }
        }
    }
}

/// A `Map OF K TO RES V` owns its values: the scope drop walks the map's entries
/// and closes each value, so the program reaches the file close op.
#[test]
fn a_map_of_resources_closes_its_values() {
    let plan = code_for_fixture(
        "resource-collection-map-value-runtime",
        CodeTarget::LinuxX86_64,
    );
    assert!(
        closes(plan).iter().any(|call| call == FS_CLOSE),
        "a map holding an `fs::File` must close it when its scope ends"
    );
}

/// A concrete resource stored into a module-level slot typed by a union is closed
/// by the union's tag dispatch at module teardown, so BOTH variants' close ops are
/// reachable from the drain.
#[test]
fn a_union_typed_module_slot_closes_by_variant_at_teardown() {
    let plan = code_for_fixture(
        "bug1027_member_into_module_union_rt",
        CodeTarget::LinuxX86_64,
    );
    let drain = plan
        .functions
        .iter()
        .find(|function| function.name == "runtime.module.resource_drain")
        .expect("a program with module-level resources has a module drain");
    let calls = Stream::of(drain).calls_between(0, drain.instructions.len());
    assert!(
        calls.iter().any(|call| call == FS_CLOSE) && calls.iter().any(|call| call == TCP_CLOSE),
        "the module drain must dispatch on a union node's variant; it called {calls:?}"
    );
}

/// Every shape of module-level store lowers on every backend, and a store into a
/// binding that holds no resource emits nothing for the module instance.
#[test]
fn module_level_stores_lower_and_a_resource_free_store_adds_no_list_work() {
    for target in CodeTarget::ALL {
        if let Err(err) = try_code_for_src(MODULE_STORES, target, Console) {
            panic!("module stores must lower on {}: {err}", target.name());
        }
    }
    let plan =
        crate::testutil::code_for_src_cached(MODULE_STORES, CodeTarget::LinuxX86_64, Console);
    let list_pushes = |name: &str| {
        let function = crate::testutil::code_function(plan, name);
        Stream::of(function)
            .calls_between(0, function.instructions.len())
            .iter()
            .filter(|call| call.contains("arena_alloc"))
            .count()
    };
    assert_eq!(
        list_pushes("bump"),
        0,
        "`counter = counter + 1` stores an Integer: no module-list node to allocate"
    );
    assert!(
        list_pushes("fromCall") > 0,
        "a call result stored into a resource global has no name to walk before the \
         store, so the whole global is walked after it and each resource listed"
    );
}

/// Resource unions aliased, wrapped from a concrete resource, returned and floated into a
/// collection lower on every backend, and the binding that finally owns them closes by
/// variant.
#[test]
fn union_aliases_lower_and_the_owning_binding_closes_by_variant() {
    for target in CodeTarget::ALL {
        if let Err(err) = try_code_for_src(UNION_ALIASES, target, Console) {
            panic!("union aliases must lower on {}: {err}", target.name());
        }
    }
    let plan =
        crate::testutil::code_for_src_cached(UNION_ALIASES, CodeTarget::LinuxX86_64, Console);
    let closes_in = |name: &str| {
        let function = crate::testutil::code_function(plan, name);
        Stream::of(function)
            .calls_between(0, function.instructions.len())
            .into_iter()
            .filter(|call| call == FS_CLOSE || call == TCP_CLOSE)
            .count()
    };
    assert!(
        closes_in("main") >= 2,
        "main owns the unions it bound and closes each by variant (File and Socket)"
    );
}
