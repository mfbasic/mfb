//! The module assembly's own refusals, driven directly.
//!
//! `lower_module_for_platform` hands its regions (`module_helpers.rs`,
//! `module_finish.rs`) tables it built from a well-formed module, so a
//! whole-program lowering never shows them inconsistent input. Each region still
//! checks the invariants it relies on and refuses, by name, when one does not
//! hold — a closure environment recorded with two layouts, a drop walker whose
//! kinds do not extend the copy walker's, a helper raising through
//! `_mfb_rt_fail_status` whose call sites carry no re-stamp. The alternative to
//! each refusal is a wrong program: one routine for two layouts, a kind index
//! that means different types to the two walkers, an error that loses its
//! location. These tests hand each region exactly the input its check exists for
//! and pin the refusal, beside the well-formed input it accepts.
//!
//! They live here rather than in a `builder/tests/` child module because a
//! `#[cfg(test)] mod` line in a production file worsens that file's own report
//! (planning/tests.md, Corrections C3); the regions are `pub(crate)` for them.

use crate::codegen::engine::builder::module_finish::{
    add_relocation_gated_data_objects, lower_relocation_gated_helpers,
};
use crate::codegen::engine::builder::module_helpers::lower_graph_and_closure_helpers;
use crate::codegen::engine::builder::{ClosureEnvLayout, ClosureEnvWord, GlobalValue, TypeModel};
use crate::codegen::engine::types::{
    CodeDataObject, CodeFrame, CodeFunction, CodeRelocation, CodegenPlatform, RelocIntent,
};
use crate::codegen::error::constants::FAIL_STATUS_SYMBOL;
use crate::codegen::memory::arena::closure_ops::ClosureHelperTables;
use crate::os::linux::flavor::LinuxFlavor::Glibc;
use crate::target::shared::nir::{NirFunction, NirModule};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;
use std::collections::HashMap;

pub(super) fn linux_x86_64() -> impl CodegenPlatform {
    crate::target::linux_common::code::Platform::for_test(
        crate::target::linux_x86_64::code::X86_64,
        Glibc,
    )
}

pub(super) fn module(source: &str) -> NirModule {
    nir_for_src(source, CodeTarget::LinuxX86_64, Console)
        .unwrap_or_else(|err| panic!("the probe lowers to NIR: {err}"))
}

/// Run `body` with the tables `lower_module_for_platform` would build for
/// `module` (its functions and string table; no imports, no globals), the
/// module's own type model and a Linux x86-64 platform.
fn with_tables<R>(
    module: &NirModule,
    body: impl FnOnce(&ClosureHelperTables<'_>, &TypeModel) -> R,
) -> R {
    crate::codegen::engine::mir::set_backend(linux_x86_64().backend());
    let platform = linux_x86_64();
    let function_symbols: HashMap<String, String> = module
        .functions
        .iter()
        .map(|function| {
            (
                function.name.clone(),
                crate::target::shared::nir::function_symbol(&function.name),
            )
        })
        .collect();
    let functions: HashMap<String, &NirFunction> = module
        .functions
        .iter()
        .map(|function| (function.name.clone(), function))
        .collect();
    let package_return_types = HashMap::new();
    let platform_imports = HashMap::new();
    let globals: HashMap<String, GlobalValue> = HashMap::new();
    let type_model = TypeModel::from_module_and_packages(module, &[])
        .unwrap_or_else(|err| panic!("the probe's type model builds: {err}"));
    let string_symbols = crate::codegen::memory::data::string_symbols(module, &type_model);
    let tables = ClosureHelperTables {
        function_symbols: &function_symbols,
        functions: &functions,
        package_return_types: &package_return_types,
        platform_imports: &platform_imports,
        platform: &platform,
        build_mode: Console,
        globals: &globals,
        string_symbols: &string_symbols,
    };
    body(&tables, &type_model)
}

const PLAIN: &str = "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n";

/// A program with a recursive union, so the type model has graph kinds.
const RECURSIVE: &str = "\
IMPORT io

TYPE Leaf
  v AS Integer
END TYPE

TYPE Node2
  left AS Tree
  right AS Tree
END TYPE

UNION Tree
  Leaf
  Node2
END UNION

FUNC main() AS Integer
  LET t AS Tree = Node2[left := Leaf[v := 1], right := Leaf[v := 2]]
  MATCH t
    CASE Leaf(l)
      io::print(toString(l.v))
    CASE Node2(n)
      io::print(\"node\")
  END MATCH
  RETURN 0
END FUNC
";

fn layout(symbol: &str, words: Vec<ClosureEnvWord>) -> ClosureEnvLayout {
    ClosureEnvLayout {
        symbol: symbol.to_string(),
        words,
        auxiliary_words: 0,
        resource_words: Vec::new(),
    }
}

/// bug-781: one environment routine is emitted per capturing `LAMBDA`, so every
/// function that creates that `LAMBDA` must have recorded the same layout. The
/// same layout twice is one routine; two different layouts under one symbol would
/// be one routine copying and dropping the other's words wrongly, and is refused
/// naming both.
#[test]
fn a_closure_environment_recorded_with_two_layouts_is_refused() {
    let module = module(PLAIN);
    with_tables(&module, |tables, type_model| {
        let owned = layout(
            "_mfb_closure_env_probe",
            vec![ClosureEnvWord::Owned(ParameterType::String)],
        );
        let mut code_functions = Vec::new();
        let mut data_objects = Vec::new();
        lower_graph_and_closure_helpers(
            tables,
            type_model,
            vec![owned.clone(), owned.clone()],
            &mut code_functions,
            &mut data_objects,
        )
        .expect("the same layout twice is one routine");
        assert_eq!(
            code_functions
                .iter()
                .filter(|function| function.symbol == "_mfb_closure_env_probe")
                .count(),
            1,
            "one routine per environment symbol"
        );

        let other = layout(
            "_mfb_closure_env_probe",
            vec![ClosureEnvWord::Owned(ParameterType::Integer)],
        );
        let err = lower_graph_and_closure_helpers(
            tables,
            type_model,
            vec![owned.clone(), other.clone()],
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .expect_err("two layouts under one symbol must be refused");
        assert_eq!(
            err,
            format!(
                "closure environment `_mfb_closure_env_probe` is created with two layouts: \
                 {owned:?} and {other:?}"
            )
        );
    });
}

/// plan-134-H: a kind index is shared by the per-type copy shims and both graph
/// walkers, so the drop walker's kinds must start with exactly the copy walker's.
/// A type model whose drop kinds do not is refused before either walker is
/// emitted; the module's own model is accepted and gets both walkers.
#[test]
fn graph_drop_kinds_that_do_not_extend_the_copy_kinds_are_refused() {
    let module = module(RECURSIVE);
    with_tables(&module, |tables, type_model| {
        assert!(
            !type_model.graph_drop_kinds.is_empty(),
            "the probe's model has graph kinds"
        );
        let mut code_functions = Vec::new();
        lower_graph_and_closure_helpers(
            tables,
            type_model,
            Vec::new(),
            &mut code_functions,
            &mut Vec::new(),
        )
        .expect("the module's own model is consistent");
        for walker in [
            crate::codegen::memory::arena::graph_copy::GRAPH_COPY_SYMBOL,
            crate::codegen::memory::arena::graph_drop::GRAPH_DROP_SYMBOL,
        ] {
            assert!(
                code_functions
                    .iter()
                    .any(|function| function.symbol == walker),
                "{walker} is emitted for a recursive type"
            );
        }

        for corrupt in [Vec::new(), vec!["NotTheCopyKind".to_string()]] {
            let mut model = type_model.clone();
            model.graph_drop_kinds = corrupt.clone();
            let err = lower_graph_and_closure_helpers(
                tables,
                &model,
                Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
            )
            .expect_err("drop kinds that do not start with the copy kinds are refused");
            assert_eq!(
                err, "the graph drop kinds do not start with the graph copy kinds",
                "drop kinds {corrupt:?}"
            );
        }
    });
}

/// A code function named `symbol` with one relocation to `to`.
fn relocating(symbol: &str, to: &str, kind: RelocIntent, binding: &str) -> CodeFunction {
    CodeFunction {
        name: symbol.to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        instructions: Vec::new(),
        relocations: vec![CodeRelocation {
            from: symbol.to_string(),
            to: to.to_string(),
            kind,
            binding: binding.to_string(),
            library: None,
        }],
        stack_slots: Vec::new(),
    }
}

/// plan-158-A: only a helper of a `PARKING_HELPER_PACKAGES` package may raise
/// through `_mfb_rt_fail_status`, because only its call sites re-stamp a parked
/// error with their own location. An `audio` helper that does gets the status
/// helper emitted; any other caller is refused by name.
#[test]
fn only_a_parking_package_helper_may_raise_through_fail_status() {
    let module = module(PLAIN);
    with_tables(&module, |tables, type_model| {
        let mut code_functions = vec![relocating(
            "_mfb_rt_audio_audio_write",
            FAIL_STATUS_SYMBOL,
            RelocIntent::Call,
            "internal",
        )];
        lower_relocation_gated_helpers(tables, type_model, &[], &mut code_functions)
            .expect("an audio helper may park an error");
        assert!(
            code_functions
                .iter()
                .any(|function| function.symbol == FAIL_STATUS_SYMBOL),
            "the status helper is emitted for its caller"
        );

        let mut code_functions = vec![relocating(
            "_mfb_rt_fs_fs_open",
            FAIL_STATUS_SYMBOL,
            RelocIntent::Call,
            "internal",
        )];
        let err = lower_relocation_gated_helpers(tables, type_model, &[], &mut code_functions)
            .expect_err("a non-parking helper must not raise through fail_status");
        assert_eq!(
            err,
            format!(
                "`_mfb_rt_fs_fs_open` raises through `{FAIL_STATUS_SYMBOL}`, but its package \
                 is not in PARKING_HELPER_PACKAGES, so its call sites do not re-stamp a \
                 parked error"
            )
        );
    });
}

fn symbols(objects: &[CodeDataObject]) -> Vec<&str> {
    objects
        .iter()
        .map(|object| object.symbol.as_str())
        .collect()
}

/// The Unicode tables are emitted per relocation, and when the module's NIR uses
/// a Unicode-table operation but no function names a specific table the whole set
/// is emitted rather than none — an undefined table symbol is a link failure,
/// extra tables are only size.
#[test]
fn unicode_use_with_no_table_relocation_emits_the_whole_table_set() {
    let module = module(
        "IMPORT strings\nIMPORT io\n\nFUNC main() AS Integer\n  LET s = io::readLine()\n  \
         io::print(strings::upper(s))\n  RETURN 0\nEND FUNC\n",
    );
    assert!(
        crate::codegen::engine::analysis::module_uses_unicode_runtime_tables(&module),
        "the probe uses a Unicode table at run time"
    );
    let mut data_objects = Vec::new();
    add_relocation_gated_data_objects(&module, &[], &mut data_objects)
        .expect("no relocations is well-formed");
    let full = crate::codegen::memory::data::unicode_runtime_data_objects(None);
    assert!(!full.is_empty());
    assert_eq!(symbols(&data_objects), symbols(&full));

    // Without the Unicode use, nothing is emitted for it.
    let mut data_objects = Vec::new();
    add_relocation_gated_data_objects(&module_plain(), &[], &mut data_objects)
        .expect("no relocations is well-formed");
    assert!(data_objects.is_empty(), "{:?}", symbols(&data_objects));
}

fn module_plain() -> NirModule {
    module(PLAIN)
}

/// plan-158-A: each `_mfb_str_errstatus_*` prefix symbol encodes its own text, so
/// the data object is decoded from the symbol. A well-formed one becomes the
/// registered message and the call; a symbol that does not decode is refused by
/// name rather than emitted as some other text.
#[test]
fn an_error_status_prefix_symbol_is_decoded_or_refused() {
    use crate::codegen::memory::data::error_status_prefix_symbol;
    let module = module_plain();
    let good = error_status_prefix_symbol("ErrResourceClosed", "fs.close");
    let mut data_objects = Vec::new();
    add_relocation_gated_data_objects(
        &module,
        &[relocating(
            "_mfb_rt_probe",
            &good,
            RelocIntent::DataAddrHi,
            "data",
        )],
        &mut data_objects,
    )
    .expect("a well-formed prefix symbol decodes");
    assert_eq!(symbols(&data_objects), vec![good.as_str()]);

    let bad = format!(
        "{}ErrResourceClosed_zz",
        crate::codegen::error::constants::ERROR_STATUS_PREFIX_SYMBOL_PREFIX
    );
    let err = add_relocation_gated_data_objects(
        &module,
        &[relocating(
            "_mfb_rt_probe",
            &bad,
            RelocIntent::DataAddrHi,
            "data",
        )],
        &mut Vec::new(),
    )
    .expect_err("a prefix symbol that does not decode is refused");
    assert_eq!(err, format!("malformed error-status prefix symbol `{bad}`"));
}
