use super::*;
use crate::types::ParameterType;

mod inline_trap_short_circuit;

/// Shared source-driven test helpers (plan-12 IR coverage). These write a
/// throwaway project to a temp dir and run the real front-end pipeline
/// (parse → resolve → monomorph → **lower**) so tests can assert on the
/// `IrProject` a real program lowers to, rather than hand-building IR.
#[cfg(test)]
pub(crate) mod helpers {
    use super::super::*;
    use crate::ast;
    use crate::manifest::validate_project_manifest;
    use crate::monomorph;
    use crate::resolver;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("timestamp")
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "mfb_ir_test_{tag}_{}_{stamp}_{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("temp dir");
        root
    }

    /// The project.json for a throwaway single-file `project`. Identical for
    /// every harness caller except the project name (bug-342 C1 folded the two
    /// hand-copied manifests — `irtest`/`irlower` — into this one builder).
    fn manifest_for(project: &str) -> String {
        format!(
            r#"{{
  "name": "{project}",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "sources": [{{ "root": "src", "role": "main", "include": ["**/*.mfb"] }}],
  "entry": "main",
  "targets": ["native"]
}}"#
        )
    }

    /// The single source-driven lower pipeline shared by every harness in this
    /// file (bug-342 C1). Writes `src` as a throwaway project named `project`
    /// under a **per-call-unique** temp dir (via `unique_dir`, so two calls
    /// never wipe each other's directory), then runs
    /// parse→resolve→monomorph→lower, returning `None` if any stage rejects the
    /// program. Panic-hook handling is the caller's job (the fallible wrappers
    /// silence it; the infallible ones let a genuine panic surface).
    fn run_lower(project: &str, tag: &str, src: &str) -> Option<IrProject> {
        let dir = unique_dir(tag);
        let result = (|| {
            std::fs::write(dir.join("project.json"), manifest_for(project)).ok()?;
            std::fs::write(dir.join("src").join("main.mfb"), src).ok()?;
            let manifest = validate_project_manifest(&dir.join("project.json")).ok()?;
            let astp = ast::parse_project(project, &dir, &manifest).ok()?;
            resolver::resolve_project(&dir, &manifest, &astp).ok()?;
            let concrete =
                monomorph::monomorphize_project(&dir, &crate::hir::elaborate(&astp)).ok()?;
            resolver::resolve_hir_project(&dir, &manifest, &concrete, false).ok()?;
            Some(crate::ir::lower_monomorphized_project(
                &concrete,
                &astp,
                None,
                &std::collections::HashMap::new(),
                &[],
                &[],
            ))
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    /// Parse+resolve+monomorph+lower a single-file program source. Panics if any
    /// stage fails, so a test failure points at genuinely broken source.
    pub(crate) fn lower_src(src: &str) -> IrProject {
        run_lower("irtest", "lower", src).expect("source must lower cleanly")
    }

    /// Like [`lower_src`] but returns `None` when any front-end stage before
    /// lowering rejects the program (so a test can assert lowering is reached).
    /// Front-end diagnostics are silenced while trying.
    pub(crate) fn try_lower_src(src: &str) -> Option<IrProject> {
        crate::testutil::silence_panics(|| run_lower("irtest", "try", src))
    }

    /// [`lower_src`] for a caller that needs a specific project `name` (so a
    /// name-dependent assertion sees it) and a descriptive temp-dir `tag`.
    pub(crate) fn lower_src_named(name: &str, tag: &str, src: &str) -> IrProject {
        run_lower(name, tag, src).expect("source must lower cleanly")
    }

    /// [`try_lower_src`] with an explicit project `name` and temp-dir `tag`.
    pub(crate) fn try_lower_src_named(name: &str, tag: &str, src: &str) -> Option<IrProject> {
        crate::testutil::silence_panics(|| run_lower(name, tag, src))
    }

    /// The named function's body in a lowered project.
    pub(crate) fn function<'a>(ir: &'a IrProject, name: &str) -> &'a IrFunction {
        ir.functions
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("function `{name}` not found in lowered project"))
    }

    #[test]
    fn lower_src_smoke() {
        let ir = lower_src("FUNC main() AS Integer\n  RETURN 1\nEND FUNC\n");
        assert_eq!(function(&ir, "main").returns.name(), "Integer");
    }
}

/// plan-20-D: IR lowering must be **total** — it never panics on ill-typed but
/// parse/resolve-clean input. The 371 `*-invalid` fixtures are exactly such
/// programs (they parse and resolve but fail a semantic rule). Today the AST
/// type checker rejects them before lowering; once the checker moves onto the
/// IR (plan-20-Z) lowering runs first, so it must survive them. This test
/// drives every `*-invalid` fixture through parse → resolve → monomorph →
/// **lower** (skipping the former source checker) and asserts lowering does not panic. Fixtures
/// that fail before lowering (parse/resolve/monomorph errors — also pre-lowering
/// rejections) are skipped; the assertion is purely "if it reaches lowering, it
/// does not panic".
#[cfg(test)]
mod lowering_totality_tests {
    use crate::ast;
    use crate::manifest::validate_project_manifest;
    use crate::monomorph;
    use crate::resolver;
    use std::path::{Path, PathBuf};

    fn invalid_fixture_dirs() -> Vec<PathBuf> {
        // Walk recursively: after the tests reorganization the `*-invalid`
        // fixtures live nested under tests/syntax/<feature>/* (and a few under
        // the runtime buckets), not directly under tests/.
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                // Compile-time-invalid fixtures reach lowering only if they
                // parse and resolve; runtime-invalid (`*-invalid-rt`) fixtures
                // are valid programs that fail at run time, so they lower too.
                if entry.file_name().to_string_lossy().ends_with("-invalid")
                    && path.join("project.json").is_file()
                {
                    out.push(path.clone());
                }
                walk(&path, out);
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let mut dirs = Vec::new();
        walk(&root, &mut dirs);
        dirs.sort();
        dirs
    }

    /// Silence the diagnostics the front end prints for invalid fixtures so the
    /// test output stays readable; we only care whether lowering panics.
    fn lower_fixture_without_panic(dir: &Path) -> Result<bool, ()> {
        let manifest = validate_project_manifest(&dir.join("project.json"))?;
        let name = manifest
            .get("name")
            .and_then(|v| v.get::<String>())
            .cloned()
            .ok_or(())?;
        let ast = ast::parse_project(&name, dir, &manifest)?;
        resolver::resolve_project(dir, &manifest, &ast)?;
        let concrete = monomorph::monomorphize_project(dir, &crate::hir::elaborate(&ast))?;
        resolver::resolve_hir_project(dir, &manifest, &concrete, false)?;
        // Reached lowering: it must not panic. Entry + external package
        // functions are irrelevant to totality, so pass the minimal inputs.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::ir::lower_monomorphized_project(
                &concrete,
                &ast,
                None,
                &std::collections::HashMap::new(),
                &[],
                &[],
            );
        }));
        Ok(result.is_ok())
    }

    #[test]
    fn lowering_is_total_over_invalid_fixtures() {
        // Suppress the front end's diagnostic noise (invalid fixtures print
        // many errors on the way to the resolve/monomorph gate).
        let (reached, panicked) = crate::testutil::silence_panics(|| {
            let mut reached = 0usize;
            let mut panicked = Vec::new();
            for dir in invalid_fixture_dirs() {
                match lower_fixture_without_panic(&dir) {
                    Ok(true) => reached += 1,
                    Ok(false) => panicked.push(dir.display().to_string()),
                    Err(()) => {} // rejected before lowering — not our concern
                }
            }
            (reached, panicked)
        });
        assert!(
            panicked.is_empty(),
            "IR lowering panicked on {} fixture(s) (not total): {:?}",
            panicked.len(),
            panicked
        );
        // Sanity: a meaningful number of fixtures actually reached lowering,
        // so the test is exercising the total paths, not vacuously passing.
        assert!(
            reached > 50,
            "only {reached} invalid fixtures reached lowering; expected the \
             source-checked majority — the pipeline wiring may be broken"
        );
    }
}

#[cfg(test)]
mod binary_repr_tests;

/// Structural coverage for `ir/lower.rs` (plan-12): valid MFBASIC programs that
/// exercise every statement / expression / type / binding / native-LINK / DOC
/// lowering path, asserting on the produced `IrProject`. Uses the shared
/// `helpers` harness so the real front-end pipeline (parse → resolve →
/// monomorph → lower) produces the IR under test.
#[cfg(test)]
mod lower_tests;

/// Per-construct lowering tests (plan-68-E1): each drives `lower_src` of a
/// snippet that forces a specific under-exercised arm of `lower_expression` /
/// `lower_statement` and asserts on the emitted `IrValue`/`IrOp` shape.
#[cfg(test)]
mod lower_construct_tests {
    use super::super::*;
    use super::helpers::{function, lower_src};

    /// The value bound by `LET <name> = …` in `func`.
    fn bind_value<'a>(ir: &'a IrProject, func: &str, name: &str) -> &'a IrValue {
        function(ir, func)
            .body
            .iter()
            .find_map(|op| match op {
                IrOp::Bind {
                    name: n,
                    value: Some(v),
                    ..
                } if n == name => Some(v),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no binding `{name}` in `{func}`"))
    }

    // The most-negative `Fixed` / `Money` literals overflow their positive
    // magnitude, so `lower_expression` folds the unary minus into the literal
    // rather than leaving a `Unary` that codegen would materialize and negate
    // at runtime (bug-07 / plan-29-B §4.2).
    #[test]
    fn negated_min_fixed_and_money_literals_fold_into_const() {
        let ir = lower_src(
            "FUNC f() AS Integer\n\
               LET a AS Fixed = -2147483648.0\n\
               LET c AS Money = -92233720368547.75808\n\
               RETURN 0\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
        );
        match bind_value(&ir, "f", "a") {
            IrValue::Const { type_, value } => {
                assert_eq!(type_.name(), "Fixed");
                assert_eq!(value, "-2147483648.0");
            }
            _ => panic!("expected a folded Fixed const"),
        }
        match bind_value(&ir, "f", "c") {
            IrValue::Const { type_, value } => {
                assert_eq!(type_.name(), "Money");
                assert!(value.starts_with('-'), "money value: {value}");
            }
            _ => panic!("expected a folded Money const"),
        }
    }

    // An unsuffixed decimal literal coerces to the slot's expected numeric type
    // (`Money`/`Fixed`), and an integer literal coerces to a `Byte` slot — the
    // expected-type arms of the number lowering.
    #[test]
    fn numeric_literals_coerce_to_expected_slot_type() {
        let ir = lower_src(
            "FUNC f() AS Integer\n\
               LET m AS Money = 1.25\n\
               LET x AS Fixed = 2.5\n\
               LET b AS Byte = 200\n\
               RETURN 0\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
        );
        for (name, want) in [("m", "Money"), ("x", "Fixed"), ("b", "Byte")] {
            match bind_value(&ir, "f", name) {
                IrValue::Const { type_, .. } => assert_eq!(type_.name(), want, "`{name}`"),
                _ => panic!("expected a {want} const for `{name}`"),
            }
        }
    }

    // A `Set OF T { … }` literal lowers each element with the set's element type
    // as the expected type, producing an `IrValue::SetLiteral`.
    #[test]
    fn set_literal_lowers_to_set_value() {
        let ir = lower_src(
            "FUNC f() AS Integer\n\
               LET s AS Set OF Integer = Set OF Integer { 1, 2, 3 }\n\
               RETURN 0\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
        );
        match bind_value(&ir, "f", "s") {
            IrValue::SetLiteral { type_, values } => {
                assert_eq!(type_.name(), "Set OF Integer");
                assert_eq!(values.len(), 3);
            }
            _ => panic!("expected a SetLiteral"),
        }
    }

    // A general `is*` predicate named in a value position expecting
    // `FUNC(T) AS Boolean` lowers to a `FunctionRef` (bug-368;
    // builtin_predicate_ref_type / function_type_parts_for_predicate).
    #[test]
    fn builtin_predicate_in_value_position_is_function_ref() {
        let ir = lower_src(
            "FUNC f() AS Integer\n\
               LET p AS FUNC(Integer) AS Boolean = isEven\n\
               RETURN 0\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
        );
        assert!(
            matches!(bind_value(&ir, "f", "p"), IrValue::FunctionRef { .. }),
            "isEven in a FUNC(Integer) AS Boolean slot must lower to a FunctionRef"
        );
    }

    // A lambda whose body references an outer local inside a `Set OF` literal
    // captures that local — `collect_captured_locals` must recurse into the set
    // literal's elements to find it.
    #[test]
    fn lambda_capturing_local_inside_set_literal_is_closure() {
        let ir = lower_src(
            "FUNC f() AS Integer\n\
               LET base AS Integer = 5\n\
               LET g AS FUNC(Integer) AS Set OF Integer = LAMBDA(v AS Integer) -> Set OF Integer { base, v }\n\
               RETURN 0\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
        );
        assert!(
            matches!(bind_value(&ir, "f", "g"), IrValue::Closure { captures, .. } if !captures.is_empty()),
            "the lambda must capture `base` referenced inside the set literal"
        );
    }

    // The return type of `collections::filter(xs, isEven)` bound without an
    // explicit type is inferred by walking the predicate argument
    // (expression_type's unary-callback-member path).
    #[test]
    fn filter_call_return_type_is_inferred_from_predicate() {
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC f() AS Integer\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               LET ys = collections::filter(xs, isEven)\n\
               RETURN 0\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
        );
        let bind = function(&ir, "f")
            .body
            .iter()
            .find_map(|op| match op {
                IrOp::Bind { name, type_, .. } if name == "ys" => Some(type_.clone()),
                _ => None,
            })
            .expect("ys binding");
        assert_eq!(bind.name(), "List OF Integer");
    }

    // A builtin call mixing positional and named arguments drives
    // `normalize_overloaded_builtin_call_arguments`'s ordering arms.
    #[test]
    fn named_and_positional_builtin_args_normalize() {
        let ir = lower_src(
            "IMPORT strings\n\
             FUNC f() AS String\n\
               RETURN strings::replace(\"aaa\", old := \"a\", new := \"b\")\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
        );
        assert!(function(&ir, "f")
            .body
            .iter()
            .any(|op| matches!(op, IrOp::Return { value: Some(_), .. })));
    }

    // A numeric literal bound with no expected type takes its type from the
    // literal classification alone (the no-expected fallback arm).
    #[test]
    fn bare_numeric_literals_type_from_classification() {
        let ir = lower_src(
            "FUNC f() AS Integer\n\
               LET raw = 3.14\n\
               RETURN 0\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
        );
        match bind_value(&ir, "f", "raw") {
            IrValue::Const { type_, .. } => assert_eq!(type_.name(), "Float"),
            _ => panic!("expected a Float const for a bare decimal literal"),
        }
    }

    // The `error(code, message)` language builtin lowers to ordinary `Error`
    // record construction stamped with the call site.
    #[test]
    fn error_builtin_lowers_to_record_construction() {
        let ir = lower_src(
            "FUNC f() AS Error\n\
               LET e AS Error = error(404, \"missing\")\n\
               RETURN e\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
        );
        // The binding produced some value (a lowered Error record), not a raw call.
        assert!(function(&ir, "f")
            .body
            .iter()
            .any(|op| matches!(op, IrOp::Bind { name, .. } if name == "e")));
    }
}

/// Source-driven tests that exercise the AST→IR lowering paths (`lower.rs`)
/// directly, asserting on the lowered `IrProject` a real program produces.
/// (bug-342 C1 repaired this truncated doc and folded this module's hand-copied
/// harness onto the shared, per-call-unique `helpers` pipeline.)
#[cfg(test)]
mod lower_pipeline_tests;

/// plan-12 IR coverage: the AttributedString-routing and rarer literal/trap
/// lowering branches are exercised in-tree only by `rt-behavior` fixtures that
/// build via the release `mfb` subprocess, so they never register under
/// `cargo llvm-cov --bin mfb`. Drive them in-process here.
#[cfg(test)]
mod lower_coverage_tests {
    use super::helpers::{function, lower_src, try_lower_src};

    #[test]
    fn astrings_tier_a_and_tier_b_calls_lower_through_the_routing() {
        // Tier-A queries (find/contains) wrap the AttributedString arg in
        // `toString`; Tier-B transforms (upper/padLeft/replace) route to the
        // `__astrings_*` source-companion bodies; padLeft's 2-arg form fills the
        // default padChar. Also drives a trapped call expression.
        let ir = lower_src(
            "IMPORT astrings\n\
             IMPORT strings\n\
             FUNC main AS Integer\n  \
               LET a AS AttributedString = astrings::fromString(\"hello world\")\n  \
               LET has AS Boolean = strings::contains(a, \"ell\")\n  \
               LET up AS AttributedString = strings::upper(a)\n  \
               LET pad AS AttributedString = strings::padLeft(a, 20)\n  \
               LET rep AS AttributedString = strings::replace(a, \"l\", \"L\")\n  \
               LET idx AS Integer = strings::find(a, \"z\") TRAP(e)\n    \
                 RETURN e.code\n  \
               END TRAP\n  \
               RETURN idx\nEND FUNC\n",
        );
        let _ = function(&ir, "main");
    }

    #[test]
    fn fixed_literals_lower_with_the_fixed_literal_type() {
        // A `Fixed` literal (`F` suffix) drives the `numeric::LiteralType::Fixed`
        // arm in both `lower_expression_with_expected` and `literal_expression_type`.
        // Inferred (no `AS Fixed`) so the literal's *classification* — not the
        // expected-type coercion — drives the `LiteralType::Fixed` arm in both
        // `lower_expression_with_expected` and `literal_expression_type`.
        let ir = lower_src(
            "FUNC main AS Integer\n  \
               LET f = 1.5F\n  \
               LET g = f + 2.5F\n  \
               RETURN 0\nEND FUNC\n",
        );
        let _ = function(&ir, "main");
    }

    #[test]
    fn term_draw_text_with_attributed_string_routes_to_the_attr_companion() {
        // `term::drawText(x, y, AttributedString)` routes to the
        // `__term_drawTextAttr` source-companion body (a String third argument stays
        // the native helper). Covers that `.or_else` arm plus the AttributedString
        // `IrValue::Call` return.
        let ir = lower_src(
            "IMPORT term\n\
             IMPORT astrings\n\n\
             FUNC main AS Integer\n  \
               LET a AS AttributedString = astrings::fromString(\"hi\")\n  \
               term::drawText(0, 0, a)\n  \
               term::drawText(0, 1, \"plain\")\n  \
               RETURN 0\nEND FUNC\n",
        );
        let _ = function(&ir, "main");
    }

    #[test]
    fn term_measure_text_with_attributed_string_routes_to_the_attr_companion() {
        let ir = lower_src(
            "IMPORT term\n\
             IMPORT astrings\n\n\
             FUNC main AS Integer\n  \
               LET a AS AttributedString = astrings::fromString(\"hi\")\n  \
               LET aw AS Integer = term::measureText(a)\n  \
               LET sw AS Integer = term::measureText(\"plain\")\n  \
               RETURN aw + sw\nEND FUNC\n",
        );
        let json = ir.to_json();
        assert!(
            json.contains("\"target\": \"#term_measureTextAttr\""),
            "AttributedString measurement must route through the bridge"
        );
        assert!(
            json.contains("\"target\": \"term.measureText\""),
            "String measurement must stay on the native member"
        );
    }

    #[test]
    fn term_measure_text_internal_helpers_are_not_user_callable() {
        for helper in ["measureTextRaw", "scalarWidthRaw"] {
            let source = format!(
                "IMPORT term\nFUNC main AS Integer\n  RETURN term::{helper}(\"x\")\nEND FUNC\n"
            );
            assert!(
                try_lower_src(&source).is_none(),
                "term::{helper} is internal-only and user source must not lower"
            );
        }
    }

    #[test]
    fn audio_calls_route_to_native_and_source_implementation_names() {
        // The `audio::` overloads whose body differs rewrite onto their own
        // internal runtime-helper name (openInput/read/poll — native) or their
        // source-companion body (play). Covers the audio `.or_else`
        // implementation-name arms in the call-lowering chain.
        let ir = lower_src(
            "IMPORT audio\n\n\
             FUNC main AS Integer\n  \
               RES mic AS audio::AudioInput = audio::openInput(48000, 1, 512)\n  \
               LET pcm = audio::read(mic, 256)\n  \
               LET pcm2 = audio::read(mic, 256, 100)\n  \
               LET ok AS Boolean = audio::poll(mic)\n  \
               RES spk AS audio::AudioOutput = audio::openOutput(48000, 1, 512)\n  \
               audio::play(spk, \"T120 O4 CDEFGAB\")\n  \
               RETURN 0\nEND FUNC\n",
        );
        let _ = function(&ir, "main");
    }

    /// bug-459: an explicit `tls::close(listener)` must rewrite to the internal
    /// `tls.closeListener` body, which cancels an `nw_listener`. The socket body
    /// it fell back to calls `nw_connection_cancel` on that listener, whose
    /// dispatch-queue slot is not a connection's -- a SIGSEGV on macOS.
    ///
    /// The rewrite selected on a BARE type name (`Listener`), while a built-in
    /// resource has been package-qualified end to end since bug-441, so the
    /// filter matched nothing from that commit on. Scope-drop was unaffected
    /// (the resource descriptor names `tls.closeListener` directly), which is
    /// why only an EXPLICIT close crashed.
    #[test]
    fn explicit_tls_listener_close_rewrites_to_the_listener_body() {
        let ir = lower_src(
            "IMPORT tls\n\n\
             FUNC main AS Integer\n  \
               RES l AS tls::Listener = tls::listen(\"127.0.0.1\", 0, \"c.pem\", \"k.pem\")\n  \
               RES c AS tls::Socket = tls::connect(\"example.com\", 443)\n  \
               tls::close(c)\n  \
               tls::close(l)\n  \
               RETURN 0\nEND FUNC\n",
        );
        let json = ir.to_json();
        assert!(
            json.contains("\"target\": \"tls.closeListener\""),
            "closing a tls::Listener must lower to tls.closeListener"
        );
        // The socket close is NOT rewritten: exactly one plain `tls.close`, for `c`.
        assert_eq!(
            json.matches("\"target\": \"tls.close\"").count(),
            1,
            "only the Socket close stays `tls.close`"
        );
    }

    /// The mirror: `tcp::Listener` also spells its bare name `Listener`, so a
    /// rewrite matching bare names would capture it too. `tcp::close` has one
    /// body for both handle kinds and must never reach tls's.
    #[test]
    fn closing_a_tcp_listener_does_not_reach_the_tls_listener_body() {
        let ir = lower_src(
            "IMPORT tcp\n\n\
             FUNC main AS Integer\n  \
               RES l AS tcp::Listener = tcp::listen(\"127.0.0.1\", 0)\n  \
               tcp::close(l)\n  \
               RETURN 0\nEND FUNC\n",
        );
        let json = ir.to_json();
        assert!(
            !json.contains("tls.closeListener"),
            "a tcp::Listener must not reach tls's listener body"
        );
        assert!(
            json.contains("\"target\": \"tcp.close\""),
            "tcp::close(Listener) stays tcp.close"
        );
    }
}
