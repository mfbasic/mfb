//! bug-568: what an inline `TRAP` binds — the fresh `Result` WRAPPER its lowering
//! built, not the callee's own returned block — asserted on the emitted code.
//!
//! The RSS cases in `tests/runtime/rt_scope_drop_leaks.rs` prove the leak is gone.
//! They cannot prove the fix is the right SHAPE, and here that matters more than
//! usual, because this fix REMOVES a deep copy:
//!
//! * **Keeping the copy** is the leak — 134 B on every call, with nothing red.
//! * **Removing it where the value really IS an alias** is a use-after-free: the
//!   binding would `arena_free` a block the caller still owns, and the arena turns
//!   a wrong free into "Allocation failed" at some later, unrelated allocation.
//!
//! So the copy's presence is asserted BOTH ways: gone for the trapped wrapper,
//! still there for the plain param-borrow call that needs it. Every assertion is
//! COMPARATIVE — one program against a sibling differing in exactly one way.
//!
//! Build-only `-ncode` cross-built for `linux-x86_64`, matching the sibling
//! codegen-inspection suites: ownership is target-independent codegen.

#[path = "../common/mod.rs"]
mod common;

use serde_json::Value;

const TARGET: &str = "linux-x86_64";

fn ncode(name: &str, source: &str) -> Value {
    let project = common::temp_project(name, source);
    let plan = common::build_ncode(&project, TARGET, name);
    let _ = std::fs::remove_dir_all(&project);
    plan
}

fn function<'a>(plan: &'a Value, symbol: &str) -> &'a Value {
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"].as_str() == Some(symbol))
        .unwrap_or_else(|| panic!("code plan has no function '{symbol}'"))
}

/// How many deep copies of a flat block `symbol` performs.
/// `copy_flat_block` allocates once and labels the allocation `flat_copy_alloc_ok`.
fn flat_copies(plan: &Value, symbol: &str) -> usize {
    function(plan, symbol)["instructions"]
        .as_array()
        .expect("instructions array")
        .iter()
        .filter(|instr| {
            instr["op"].as_str() == Some("label")
                && instr["name"]
                    .as_str()
                    .is_some_and(|name| name.starts_with("flat_copy_alloc_ok"))
        })
        .count()
}

fn arena_allocs(plan: &Value, symbol: &str) -> usize {
    function(plan, symbol)["instructions"]
        .as_array()
        .expect("instructions array")
        .iter()
        .filter(|instr| instr["target"].as_str() == Some("_mfb_arena_alloc"))
        .count()
}

/// `main`, looping over a fallible `risky` whose body is `body`, binding the
/// result through an inline `TRAP`.
fn trapping(body: &str, type_: &str, recover: &str) -> String {
    format!(
        "IMPORT io\n\
         FUNC risky(i AS Integer) AS {type_}\n\
        \x20 IF i < 0 THEN\n\
        \x20   FAIL error(1, \"neg\")\n\
        \x20 END IF\n\
        \x20 {body}\n\
         END FUNC\n\
         SUB main()\n\
        \x20 LET n AS {type_} = risky(3) TRAP(e)\n\
        \x20   RECOVER {recover}\n\
        \x20 END TRAP\n\
        \x20 io::print(toString(n))\n\
         END SUB\n"
    )
}

/// The defect, as a count: a callee that `RETURN`s a PARAMETER made the `TRAP`
/// bind deep-copy the whole `Result` — one extra `arena_alloc` and one extra
/// `flat_copy` per call — while the identical program whose callee computes its
/// result did not.
///
/// The two programs differ ONLY in `RETURN i` vs `RETURN i / 2`, so the delta is
/// the param-borrow verdict and nothing else. `call_returns_param_borrow` is a
/// statement about the callee's SUCCESS value; on a `CallResult` the lowered value
/// is the `{tag, size, payload}` wrapper this frame allocated instead, so the
/// verdict never applied to it.
#[test]
fn a_trapped_call_never_copies_the_result_wrapper() {
    let borrows = ncode("b568_param_borrow", &trapping("RETURN i", "Integer", "0"));
    let computes = ncode("b568_computes", &trapping("RETURN i / 2", "Integer", "0"));
    assert_eq!(
        flat_copies(&borrows, "_mfb_fn_main"),
        flat_copies(&computes, "_mfb_fn_main"),
        "a TRAP over a param-returning callee must bind the same way as one over a \
         computing callee — the value bound is the Result wrapper either way"
    );
    assert_eq!(
        arena_allocs(&borrows, "_mfb_fn_main"),
        arena_allocs(&computes, "_mfb_fn_main"),
        "and it must not allocate an extra block to copy the wrapper into"
    );
}

/// The same for a `String` payload, where the abandoned copy was 195 B rather
/// than 134 B because the `Result` inlines the whole string.
#[test]
fn a_trapped_string_call_never_copies_the_result_wrapper() {
    let borrows = ncode(
        "b568_param_borrow_str",
        "IMPORT io\n\
         FUNC pick(s AS String, i AS Integer) AS String\n\
        \x20 IF i < 0 THEN\n\
        \x20   FAIL error(1, \"neg\")\n\
        \x20 END IF\n\
        \x20 RETURN s\n\
         END FUNC\n\
         SUB main()\n\
        \x20 LET base AS String = \"abcdefghij\"\n\
        \x20 LET n AS String = pick(base, 3) TRAP(e)\n\
        \x20   RECOVER \"x\"\n\
        \x20 END TRAP\n\
        \x20 io::print(n)\n\
         END SUB\n",
    );
    let computes = ncode(
        "b568_computes_str",
        "IMPORT io\n\
         FUNC pick(s AS String, i AS Integer) AS String\n\
        \x20 IF i < 0 THEN\n\
        \x20   FAIL error(1, \"neg\")\n\
        \x20 END IF\n\
        \x20 RETURN toString(len(s))\n\
         END FUNC\n\
         SUB main()\n\
        \x20 LET base AS String = \"abcdefghij\"\n\
        \x20 LET n AS String = pick(base, 3) TRAP(e)\n\
        \x20   RECOVER \"x\"\n\
        \x20 END TRAP\n\
        \x20 io::print(n)\n\
         END SUB\n",
    );
    assert_eq!(
        flat_copies(&borrows, "_mfb_fn_main"),
        flat_copies(&computes, "_mfb_fn_main"),
    );
}

/// The NEGATIVE pin, and the reason the fix is keyed on the `CallResult` node and
/// not on the callee: without a `TRAP` the lowered value IS the callee's block, a
/// pointer into the CALLER's own argument, and the copy must stay.
///
/// Removing it there would give the binding an alias it then `arena_free`s at
/// scope drop — a use-after-free of a live local, which no leak test can see. The
/// contrast against the computing callee is what makes the count meaningful:
/// only the param-borrow program has the extra copy.
#[test]
fn a_param_borrow_without_a_trap_still_copies() {
    let borrows = ncode(
        "b568_plain_borrow",
        "IMPORT io\n\
         FUNC pick(s AS String) AS String\n\
        \x20 RETURN s\n\
         END FUNC\n\
         SUB main()\n\
        \x20 LET base AS String = \"abcdefghij\"\n\
        \x20 LET n AS String = pick(base)\n\
        \x20 io::print(n & base)\n\
         END SUB\n",
    );
    let computes = ncode(
        "b568_plain_computes",
        "IMPORT io\n\
         FUNC pick(s AS String) AS String\n\
        \x20 RETURN toString(len(s))\n\
         END FUNC\n\
         SUB main()\n\
        \x20 LET base AS String = \"abcdefghij\"\n\
        \x20 LET n AS String = pick(base)\n\
        \x20 io::print(n & base)\n\
         END SUB\n",
    );
    assert!(
        flat_copies(&borrows, "_mfb_fn_main") > flat_copies(&computes, "_mfb_fn_main"),
        "a plain (untrapped) call to a param-returning callee must still deep-copy \
         its result — the block it returns is the caller's own argument, and \
         binding it without a copy is a use-after-free at scope drop"
    );
}

/// The totality assertion behind the fix.
///
/// `is_fresh_trapped_result_wrapper` skips the callee-keyed borrow predicates for
/// any `CallResult` whose lowered type is a `Result`. That is sound only because
/// a `Result`-typed `ValueResult` is ALWAYS the block
/// `emit_build_result_inline` just allocated — so the construction is funnelled
/// through the single constructor `fresh_trapped_result_value`, and this test
/// asserts nothing else builds one. A new lowering that returned a `Result` by
/// aliasing something would have to add a second construction site, and that reds
/// here rather than silently becoming a value the binding frees.
#[test]
fn the_result_wrapper_has_exactly_one_constructor() {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut sites: Vec<String> = Vec::new();
    // Scoped to `src/codegen`, which is where `ValueResult` is built. The same
    // field spelling appears on `IrValue` in `src/ir` — a different type, whose
    // `Result` node is a lowering description and not an arena pointer at all —
    // so widening this walk would flag it and say nothing about ownership.
    let mut stack = vec![manifest.join("src").join("codegen")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read source dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).expect("read source");
            // A `ValueResult` construction whose `type_` is a `Result`: the two
            // tokens land on the same or adjacent lines in every spelling this
            // tree uses, so scan the field itself.
            for (line_no, line) in src.lines().enumerate() {
                if line.contains("type_: ParameterType::result_of(") {
                    let rel = path.strip_prefix(&manifest).unwrap().display();
                    sites.push(format!("{rel}:{}", line_no + 1));
                }
            }
        }
    }
    sites.sort();
    let files: Vec<String> = sites
        .iter()
        .map(|s| s.split(':').next().unwrap().to_string())
        .collect();
    assert_eq!(
        files,
        vec!["src/codegen/memory/arena/builder_arena_transfer.rs".to_string()],
        "a `Result`-typed ValueResult must be built only by \
         `fresh_trapped_result_value`, because bug-568's fix binds one WITHOUT a \
         deep copy on the strength of it being this frame's fresh block. Sites \
         found: {sites:?}"
    );
}
