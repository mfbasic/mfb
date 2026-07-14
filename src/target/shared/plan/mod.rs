use crate::json_string;

pub(super) use super::nir::{self, NirFunction, NirModule, NirOp, NirValue};
pub(super) use super::runtime;

pub(crate) struct NativePlan {
    pub(crate) target: String,
    /// Native build mode this plan was lowered for (`console` or `macos-app`),
    /// carried from the NIR module so the backend selects the right runtime shape.
    pub(crate) build_mode: crate::target::NativeBuildMode,
    pub(crate) project: String,
    pub(crate) entry_symbol: Option<String>,
    pub(crate) runtime_symbols: Vec<String>,
    pub(crate) external_symbols: Vec<String>,
    pub(crate) platform_imports: Vec<PlatformImport>,
    pub(crate) functions: Vec<PlannedFunction>,
    /// Internal symbols the backend defines for native `LINK` bindings: the
    /// load-time initializer (`_mfb_linker_init`) and one marshaling thunk per
    /// function (plan-linker.md §12). The object plan treats these as defined.
    pub(crate) link_symbols: Vec<String>,
}

pub(crate) struct PlatformImport {
    pub(crate) library: String,
    pub(crate) symbol: String,
    pub(crate) required_by: String,
}

/// Base libc symbol names required by each `net` runtime helper. These are
/// platform independent; macOS prepends a leading `_` (libSystem) and Linux uses
/// them verbatim (libc). The platform's `errno` accessor (`___error` /
/// `__errno_location`) is added separately because its name differs by platform.
pub(crate) fn net_libc_symbols(call: &str) -> &'static [&'static str] {
    match call {
        "net.lookup" => &["getaddrinfo", "freeaddrinfo", "inet_ntop"],
        "net.connectTcp" => &[
            "getaddrinfo",
            "freeaddrinfo",
            "socket",
            "connect",
            "close",
            "fcntl",
            "poll",
            "getsockopt",
        ],
        "net.listenTcp" => &[
            "getaddrinfo",
            "freeaddrinfo",
            "socket",
            "setsockopt",
            "bind",
            "listen",
            "close",
        ],
        "net.accept" => &["accept", "poll", "close"],
        "net.poll" => &["poll"],
        "net.read" | "net.readText" => &["read"],
        "net.write" | "net.writeText" => &["write"],
        "net.close" => &["close"],
        "net.localAddress" => &["getsockname", "inet_ntop"],
        "net.remoteAddress" => &["getpeername", "inet_ntop"],
        "net.setReadTimeout" | "net.setWriteTimeout" => &["setsockopt"],
        "net.bindUdp" => &["getaddrinfo", "freeaddrinfo", "socket", "bind", "close"],
        "net.receiveFrom" | "net.receiveTextFrom" => &["recvfrom", "inet_ntop"],
        "net.sendTo" | "net.sendTextTo" => &["getaddrinfo", "freeaddrinfo", "sendto"],
        _ => &[],
    }
}

pub(crate) struct PlannedFunction {
    pub(crate) name: String,
    pub(crate) symbol: String,
    pub(crate) returns: StorageType,
    pub(crate) params: Vec<PlannedParam>,
    pub(crate) local_slots: Vec<StackSlot>,
    pub(crate) labels: Vec<PlanLabel>,
    pub(crate) operations: Vec<String>,
    pub(crate) calls: Vec<PlanCall>,
}

pub(crate) struct PlannedParam {
    pub(crate) name: String,
    pub(crate) storage: StorageType,
}

pub(crate) struct StackSlot {
    pub(crate) name: String,
    pub(crate) storage: StorageType,
    pub(crate) offset: i32,
    pub(crate) mutable: bool,
}

pub(crate) struct PlanLabel {
    pub(crate) name: String,
    pub(crate) kind: LabelKind,
}

pub(crate) enum LabelKind {
    IfElse,
    IfEnd,
    MatchCase,
    MatchEnd,
    WhileLoop,
    WhileEnd,
}

pub(crate) struct PlanCall {
    pub(crate) target: String,
    pub(crate) symbol: String,
    pub(crate) kind: CallKind,
    pub(crate) string_literals: Vec<String>,
}

pub(crate) enum CallKind {
    Local,
    /// Never produced: `import_symbols` is always empty (bug-139.2), so no call is
    /// ever classified as an import. The variant is retained only because `src/os/`
    /// object emitters still enumerate it in an exhaustive match.
    #[allow(dead_code)]
    Import,
    Runtime,
    Indirect,
}

#[derive(Clone)]
pub(crate) struct StorageType {
    pub(crate) name: String,
    pub(crate) class: StorageClass,
    pub(crate) size: usize,
    pub(crate) align: usize,
}

#[derive(Clone)]
pub(crate) enum StorageClass {
    Void,
    Byte,
    Integer,
    Float,
    Fixed,
    /// `Money`: an 8-byte base-10 fixed-point i64 carrier (plan-29-C). A distinct
    /// class (not a reuse of `Integer`) so the immediate/const-fold path selects
    /// `money_raw_from_decimal` cleanly and future divergence stays localized.
    Money,
    Boolean,
    Reference,
}

pub(crate) trait NativePlanPlatform {
    fn target(&self) -> &'static str;
    fn entry_imports(&self, module: &NirModule) -> Vec<PlatformImport>;
    fn entry_error_imports(&self, module: &NirModule) -> Vec<PlatformImport>;
    fn program_exit_imports(&self, required_by: &str) -> Vec<PlatformImport>;
    fn runtime_imports(&self, spec: &runtime::RuntimeHelperSpec) -> Vec<PlatformImport>;
    fn native_call_imports(&self, target: &str, required_by: &str) -> Vec<PlatformImport>;
    /// The libc imports (`dlopen`/`dlsym`) the per-library `LINK` initializer
    /// needs to resolve user binding symbols at load time (plan-linker.md §12.1).
    fn link_imports(&self, required_by: &str) -> Vec<PlatformImport>;
    /// Imports the macOS app-mode `_main` bootstrap needs: the Obj-C runtime,
    /// AppKit/Foundation classes, and pthread/env primitives
    /// (plan-04-macos-app.md §6.5). Empty for targets without app mode.
    fn app_mode_imports(&self) -> Vec<PlatformImport> {
        Vec::new()
    }
}

mod function_builder;
mod json;
mod lower;
mod symbols;

pub(crate) use lower::lower_module_for_platform;

use function_builder::*;
use json::*;
use lower::*;
use symbols::*;

impl NativePlan {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.target.is_empty() {
            return Err("native plan target must not be empty".to_string());
        }
        if self.project.is_empty() {
            return Err("native plan project name must not be empty".to_string());
        }
        if self.functions.is_empty() {
            return Err("native plan requires at least one function".to_string());
        }
        if let Some(entry_symbol) = &self.entry_symbol {
            if !self
                .functions
                .iter()
                .any(|function| &function.symbol == entry_symbol)
            {
                return Err(format!(
                    "native plan entry symbol '{entry_symbol}' does not resolve"
                ));
            }
        }
        for symbol in self
            .runtime_symbols
            .iter()
            .chain(self.external_symbols.iter())
        {
            if symbol.is_empty() {
                return Err("native plan contains an empty required symbol".to_string());
            }
        }
        for import in &self.platform_imports {
            if import.library.is_empty()
                || import.symbol.is_empty()
                || import.required_by.is_empty()
            {
                return Err("native plan contains an incomplete platform import".to_string());
            }
        }
        for function in &self.functions {
            function.validate()?;
        }
        Ok(())
    }

    pub(crate) fn to_json(&self) -> String {
        let link_symbols = if self.link_symbols.is_empty() {
            String::new()
        } else {
            format!(
                "  \"linkSymbols\": [{}],\n",
                json_string_list(&self.link_symbols)
            )
        };
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-native-plan\",\n",
                "  \"version\": 1,\n",
                "  \"target\": {},\n",
                "  \"buildMode\": {},\n",
                "  \"project\": {},\n",
                "  \"entrySymbol\": {},\n",
                "  \"runtimeSymbols\": [{}],\n",
                "  \"externalSymbols\": [{}],\n",
                "{}",
                "  \"platformImports\": [{}\n  ],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.target),
            json_string(self.build_mode.as_str()),
            json_string(&self.project),
            self.entry_symbol
                .as_ref()
                .map(|symbol| json_string(symbol))
                .unwrap_or_else(|| "null".to_string()),
            json_string_list(&self.runtime_symbols),
            json_string_list(&self.external_symbols),
            link_symbols,
            join_json(&self.platform_imports, 2),
            join_json(&self.functions, 2)
        )
    }
}

impl PlannedFunction {
    fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() || self.symbol.is_empty() {
            return Err("native plan function name and symbol must not be empty".to_string());
        }
        self.returns.validate()?;
        for param in &self.params {
            if param.name.is_empty() {
                return Err(format!(
                    "native plan function '{}' has an empty parameter name",
                    self.name
                ));
            }
            param.storage.validate()?;
        }
        for slot in &self.local_slots {
            if slot.name.is_empty() {
                return Err(format!(
                    "native plan function '{}' has an empty stack slot name",
                    self.name
                ));
            }
            slot.storage.validate()?;
            if slot.offset >= 0 {
                return Err(format!(
                    "native plan stack slot '{}' in '{}' has non-stack offset {}",
                    slot.name, self.name, slot.offset
                ));
            }
            let _is_mutable = slot.mutable;
        }
        for label in &self.labels {
            if label.name.is_empty() {
                return Err(format!(
                    "native plan function '{}' has an empty label name",
                    self.name
                ));
            }
            match label.kind {
                LabelKind::IfElse
                | LabelKind::IfEnd
                | LabelKind::MatchCase
                | LabelKind::MatchEnd
                | LabelKind::WhileLoop
                | LabelKind::WhileEnd => {}
            }
        }
        if self.operations.is_empty() {
            return Err(format!(
                "native plan function '{}' has no planned operations",
                self.name
            ));
        }
        for call in &self.calls {
            if call.target.is_empty() {
                return Err(format!(
                    "native plan function '{}' has an empty call target",
                    self.name
                ));
            }
            match call.kind {
                CallKind::Local | CallKind::Import | CallKind::Runtime => {
                    if call.symbol.is_empty() {
                        return Err(format!(
                            "native plan function '{}' has a call to '{}' with an empty symbol",
                            self.name, call.target
                        ));
                    }
                }
                CallKind::Indirect => {
                    // An indirect call dispatches through a runtime value and has
                    // no linker symbol; carrying one would be a lie the object
                    // plan could turn into a bogus relocation (bug-72).
                    if !call.symbol.is_empty() {
                        return Err(format!(
                            "native plan function '{}' has an indirect call to '{}' that carries a linker symbol '{}'",
                            self.name, call.target, call.symbol
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

impl StorageType {
    fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("native plan storage type name must not be empty".to_string());
        }
        match self.class {
            StorageClass::Void => {
                if self.size != 0 || self.align != 1 {
                    return Err(format!(
                        "native plan void storage '{}' must be size 0 align 1",
                        self.name
                    ));
                }
            }
            StorageClass::Boolean
            | StorageClass::Byte
            | StorageClass::Integer
            | StorageClass::Float
            | StorageClass::Fixed
            | StorageClass::Money
            | StorageClass::Reference => {
                if self.size == 0 || self.align == 0 {
                    return Err(format!(
                        "native plan storage '{}' must have nonzero size and alignment",
                        self.name
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::nir::{NirEntryPoint, NirFunction, NirModule, NirOp, NirValue};
    use crate::target::shared::runtime::{RuntimeHelper, RuntimeHelperSpec};

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

        fn runtime_imports(&self, spec: &RuntimeHelperSpec) -> Vec<PlatformImport> {
            match spec.call {
                "io.print" | "io.write" | "io.printError" | "io.writeError" => {
                    vec![PlatformImport {
                        library: "testRuntime".to_string(),
                        symbol: "test_output".to_string(),
                        required_by: spec.symbol.to_string(),
                    }]
                }
                "io.input" | "io.readLine" | "io.readChar" | "io.readByte" => {
                    vec![PlatformImport {
                        library: "testRuntime".to_string(),
                        symbol: "test_input".to_string(),
                        required_by: spec.symbol.to_string(),
                    }]
                }
                "io.pollInput" => vec![PlatformImport {
                    library: "testRuntime".to_string(),
                    symbol: "test_poll".to_string(),
                    required_by: spec.symbol.to_string(),
                }],
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
            stdin_log_cap: crate::target::shared::code::STDIN_LOG_CAP_DEFAULT,
            project: "hello".to_string(),
            entry: Some(NirEntryPoint {
                name: "main".to_string(),
                returns: "Nothing".to_string(),
                accepts_args: false,
            }),
            types: Vec::new(),
            globals: Vec::new(),
            imports: Vec::new(),
            runtime_helpers: vec![RuntimeHelper::Io],
            functions: vec![NirFunction {
                name: "main".to_string(),
                visibility: "private".to_string(),
                kind: "sub".to_string(),
                isolated: false,
                params: Vec::new(),
                returns: "Nothing".to_string(),
                body: vec![NirOp::Eval {
                    value: NirValue::RuntimeCall {
                        helper: RuntimeHelper::Io,
                        target: "io.print".to_string(),
                        args: vec![NirValue::Const {
                            type_: "String".to_string(),
                            value: "Hello World".to_string(),
                        }],
                        loc: nir::NirSourceLoc::default(),
                    },
                }],
                file: "src/main.mfb".to_string(),
                resource_owners: std::collections::HashMap::new(),
            }],
            link_functions: Vec::new(),
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
}
