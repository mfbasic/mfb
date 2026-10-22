//! The `MFB_TEST_GRAPH_DROP` symmetry hook, driven through its real switch.
//!
//! The hook reads its environment variable once per compiler process (`OnceLock`), and
//! every lowering in this test binary shares that process, so setting the variable here
//! would race every other test's first lowering. Instead the test re-runs ITSELF in a
//! child process of this same test binary with the variable set at start-up — exactly as
//! `tests/runtime/rt_recursive_value_drop_symmetry.rs` sets it for `mfb build` — and the
//! child asserts on what the hook emitted. (The child inherits `LLVM_PROFILE_FILE`, so an
//! instrumented run records what it executes.)

use crate::codegen::engine::tests::test_support::Stream;
use crate::testutil::{code_for_src_on, code_function, CodeTarget};

/// Set in the child: run the assertions instead of spawning.
const CHILD_ENV: &str = "MFB_TEST_GRAPH_DROP_HOOK_CHILD";

/// `tree` is a recursive local, bound once and reassigned once in `main`, and assigned
/// once more inside a `forEach` callback, where it is captured BY REFERENCE. `plain` is an
/// `Integer`, and `spare` a global.
const PROGRAM: &str = "\
IMPORT collections
IMPORT io

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

MUT spare AS Node = Node[[], 0]

FUNC main() AS Integer
  MUT tree AS Node = Node[[Node[[], 1]], 2]
  tree = Node[[tree, tree], 3]
  LET plain AS Integer = len(tree.kids)
  collections::forEach([4], LAMBDA(v AS Integer) -> tree = Node[[tree], v])
  spare = tree
  io::print(toString(tree.tag) & \" \" & toString(plain) & \" \" & toString(spare.tag))
  RETURN 0
END FUNC
";

/// How many hook sites `function` has: each is one `graph_drop_hook_absent` label.
fn hook_sites(plan: &crate::codegen::engine::types::NativeCodePlan, function: &str) -> usize {
    Stream::of(code_function(plan, function))
        .labels()
        .into_iter()
        .filter(|(_, name)| name.starts_with("graph_drop_hook_absent"))
        .count()
}

/// The child's half: with `tree,plain,spare,ghost` listed, the hook follows the `LET` and
/// the reassignment of `tree` in `main` — and nothing else: not the `Integer`, not the
/// global, not the by-reference write inside the callback (the callee does not own that
/// value), and not a name no op binds.
fn assert_hooked() {
    let plan = code_for_src_on(PROGRAM, CodeTarget::LinuxX86_64);
    assert_eq!(
        hook_sites(&plan, "main"),
        2,
        "the LET and the reassignment of `tree`"
    );
    for function in plan.functions.iter().filter(|f| f.name.contains("lambda")) {
        assert_eq!(
            hook_sites(&plan, &function.name),
            0,
            "{}: a by-reference capture is not hooked",
            function.name
        );
    }
    let main = code_function(&plan, "main");
    assert!(
        main.relocations
            .iter()
            .any(|relocation| relocation.to == super::GRAPH_DROP_SYMBOL),
        "each hook site drops its copy through the walker"
    );
}

#[test]
fn the_graph_drop_hook_follows_each_write_of_a_listed_recursive_local() {
    if std::env::var_os(CHILD_ENV).is_some() {
        assert_hooked();
        return;
    }
    // Unset — this process — the hook emits nothing.
    if std::env::var_os(super::TEST_GRAPH_DROP_ENV).is_none() {
        let plan = code_for_src_on(PROGRAM, CodeTarget::LinuxX86_64);
        assert_eq!(hook_sites(&plan, "main"), 0, "an unset hook emits nothing");
    }
    let path = module_path!()
        .split_once("::")
        .map_or(module_path!(), |(_, rest)| rest);
    let name =
        format!("{path}::the_graph_drop_hook_follows_each_write_of_a_listed_recursive_local");
    let output = std::process::Command::new(std::env::current_exe().expect("the test binary"))
        .args([
            name.as_str(),
            "--exact",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env(CHILD_ENV, "1")
        .env(super::TEST_GRAPH_DROP_ENV, " tree, plain,,spare ,ghost")
        .output()
        .expect("re-run the test binary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "the hooked child failed or did not run the test:\n{stdout}\n{stderr}"
    );
}
