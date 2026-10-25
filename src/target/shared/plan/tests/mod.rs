use super::*;

mod produced_closes;
use crate::target::shared::nir::{NirEntryPoint, NirFunction, NirModule, NirOp, NirValue};
use crate::target::shared::runtime::{RuntimeHelper, RuntimeHelperSpec};
use crate::types::ParameterType;

struct TestPlatform;

impl NativePlanPlatform for TestPlatform {
    fn target(&self) -> &'static str {
        "test-target"
    }

    fn entry_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        if module.entry.is_none() {
            return Vec::new();
        }
        vec![PlatformImport {
            library: "testRuntime".to_string(),
            symbol: "test_program_done".to_string(),
            required_by: "_main".to_string(),
        }]
    }

    fn entry_error_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        if module.entry.is_none() {
            return Vec::new();
        }
        vec![PlatformImport {
            library: "testRuntime".to_string(),
            symbol: "test_error_output".to_string(),
            required_by: "_main".to_string(),
        }]
    }

    fn program_exit_imports(&self, required_by: &str) -> Vec<PlatformImport> {
        vec![PlatformImport {
            library: "testRuntime".to_string(),
            symbol: "test_program_exit".to_string(),
            required_by: required_by.to_string(),
        }]
    }

    fn debug_env_imports(&self, required_by: &str) -> Vec<PlatformImport> {
        vec![PlatformImport {
            library: "testRuntime".to_string(),
            symbol: "test_getenv".to_string(),
            required_by: required_by.to_string(),
        }]
    }

    fn peak_rss_imports(&self, required_by: &str) -> Vec<PlatformImport> {
        vec![PlatformImport {
            library: "testRuntime".to_string(),
            symbol: "test_peak_rss".to_string(),
            required_by: required_by.to_string(),
        }]
    }

    fn runtime_imports(&self, spec: &RuntimeHelperSpec) -> Vec<PlatformImport> {
        let required_by = runtime::symbol_for_call(spec.helper, spec.call);
        match spec.call {
            "io.print" | "io.write" | "io.printError" | "io.writeError" => {
                vec![PlatformImport {
                    library: "testRuntime".to_string(),
                    symbol: "test_output".to_string(),
                    required_by: required_by.clone(),
                }]
            }
            "io.input" | "io.readLine" | "io.readRaw" | "io.readScalar" | "io.readGrapheme"
            | "io.readKey" => {
                vec![PlatformImport {
                    library: "testRuntime".to_string(),
                    symbol: "test_input".to_string(),
                    required_by: required_by.clone(),
                }]
            }
            _ => Vec::new(),
        }
    }

    fn native_call_imports(&self, _target: &str, _required_by: &str) -> Vec<PlatformImport> {
        Vec::new()
    }

    fn link_imports(&self, _required_by: &str) -> Vec<PlatformImport> {
        Vec::new()
    }
}

#[test]
fn plans_runtime_symbol_and_entry_function() {
    let module = NirModule {
        target: "test-target".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        stdin_log_cap: crate::codegen::error::constants::STDIN_LOG_CAP_DEFAULT,
        debug: crate::codegen::debug::DebugOptions::OFF,
        project: "hello".to_string(),
        entry: Some(NirEntryPoint {
            name: "main".to_string(),
            returns: ParameterType::Nothing,
            accepts_args: false,
        }),
        types: Vec::new(),
        globals: Vec::new(),
        imports: Vec::new(),
        runtime_helpers: vec![RuntimeHelper::Io],
        functions: vec![NirFunction {
            name: "main".to_string(),
            visibility: "private".to_string(),
            kind: "func".to_string(),
            isolated: false,
            params: Vec::new(),
            returns: ParameterType::Nothing,
            body: vec![NirOp::Eval {
                value: NirValue::RuntimeCall {
                    helper: RuntimeHelper::Io,
                    target: "io.print".to_string(),
                    args: vec![NirValue::Const {
                        type_: ParameterType::String,
                        value: "Hello World".to_string(),
                    }],
                    loc: nir::NirSourceLoc::default(),
                },
            }],
            file: "src/main.mfb".to_string(),
            resource_owners: std::collections::HashMap::new(),
        }],
        link_functions: Vec::new(),
        link_cstructs: Vec::new(),
        native_resources: Vec::new(),
        native_libraries: Default::default(),
        max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
        canvas_auto_paint_ms: crate::manifest::DEFAULT_CANVAS_AUTO_PAINT_MS,
        arena_memory: crate::ir::ArenaMemoryLimits::default(),
    };

    let plan = lower_module_for_platform(&module, &TestPlatform).expect("native plan");
    plan.validate().expect("valid native plan");
    assert_eq!(plan.entry_symbol.as_deref(), Some("_mfb_fn_main"));
    assert_eq!(plan.runtime_symbols, vec!["_mfb_rt_io_io_print"]);
    assert_eq!(plan.platform_imports[0].library, "testRuntime");
    assert_eq!(plan.platform_imports[0].symbol, "test_program_done");
    assert_eq!(plan.platform_imports[0].required_by, "_main");
    assert_eq!(plan.platform_imports[1].library, "testRuntime");
    assert_eq!(plan.platform_imports[1].symbol, "test_error_output");
    assert_eq!(plan.platform_imports[1].required_by, "_main");
    assert_eq!(plan.platform_imports[2].library, "testRuntime");
    assert_eq!(plan.platform_imports[2].symbol, "test_output");
    assert_eq!(plan.platform_imports[2].required_by, "_mfb_rt_io_io_print");
    assert_eq!(plan.functions[0].calls[0].symbol, "_mfb_rt_io_io_print");
}

/// A platform that implements none of the optional hooks declares no toolkit,
/// window, mouse or debug-readback imports, however app-shaped the module is.
///
/// `app_mode_imports`, `app_window_imports` and `debug_app_mode_imports` default
/// to empty -- "Empty for targets without app mode" -- and a console-only backend
/// (riscv64) relies on that: it is lowered through the same `lower_module` and
/// must not be handed an import list it cannot satisfy.
#[test]
fn a_platform_without_app_hooks_declares_no_toolkit_imports_for_an_app_module() {
    use crate::testutil::{nir_for_src, CodeTarget};
    let source = "\
IMPORT app

FUNC main() AS Integer
  app::setTitle(\"Demo\")
  RETURN 0
END FUNC
";
    let mut module = nir_for_src(
        source,
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::LinuxApp,
    )
    .expect("the program lowers to NIR");
    assert!(
        crate::codegen::builtins::app::module_uses_app_window(&module),
        "control: the module uses an app window member"
    );
    module.debug.enabled = true;
    module.target = "test-target".to_string();
    let plan = lower_module_for_platform(&module, &TestPlatform).expect("native plan");
    assert!(
        plan.platform_imports
            .iter()
            .all(|import| import.library == "testRuntime"),
        "only the platform's own imports: {:?}",
        plan.platform_imports
            .iter()
            .map(|import| import.symbol.as_str())
            .collect::<Vec<_>>()
    );
}

/// A call that is not a socket member has no libc symbols of its own.
#[test]
fn a_call_that_is_not_a_socket_member_needs_no_socket_libc_symbols() {
    assert!(net_libc_symbols("io.print").is_empty());
    assert!(net_libc_symbols("net.lookup").contains(&"getaddrinfo"));
}

/// A plan whose entry symbol names no planned function is invalid.
#[test]
fn a_plan_whose_entry_symbol_resolves_to_no_function_is_invalid() {
    let module = crate::testutil::nir_for_src(
        "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        crate::testutil::CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers to NIR");
    let mut module = module;
    module.target = "test-target".to_string();
    let mut plan = lower_module_for_platform(&module, &TestPlatform).expect("native plan");
    plan.validate().expect("the plan is valid as lowered");
    plan.entry_symbol = Some("_mfb_fn_nowhere".to_string());
    let error = plan.validate().expect_err("an unresolved entry is refused");
    assert_eq!(
        error,
        "native plan entry symbol '_mfb_fn_nowhere' does not resolve"
    );
}

/// A module lowered for one target is refused by another target's plan
/// platform, naming both, rather than planned with the wrong imports.
#[test]
fn a_module_lowered_for_another_target_is_refused() {
    use crate::testutil::{nir_for_src, CodeTarget};
    let module = nir_for_src(
        "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers to NIR");
    match lower_module_for_platform(&module, &TestPlatform) {
        Ok(_) => panic!("a linux-x86_64 module was planned for test-target"),
        Err(message) => assert_eq!(
            message,
            format!(
                "native plan platform 'test-target' cannot lower module target '{}'",
                module.target
            )
        ),
    }
}
