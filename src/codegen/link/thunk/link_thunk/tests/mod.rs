use super::*;
use crate::arch::ops::CodeOp;
use crate::codegen::engine::mir;
use crate::codegen::link::locator::ResolvedLibrary;
use crate::ir::{
    abi_ctype_valid_as_argument, abi_ctype_valid_as_return, IrAbiSlot, IrCStruct, IrCStructField,
    IrLinkFunction,
};

/// bug-409: every label-targeting branch a lowered thunk emits must name a
/// label the same thunk defines. This is the exact invariant
/// `CodeFunction::validate` enforces (`validation.rs` bug-300 E9); a dangling
/// branch here is a hard build failure ("branch target label does not
/// resolve") on an otherwise valid binding. Asserting it directly, rather
/// than through the full `validate` (which also demands populated symbol
/// tables), keeps the test focused on the label gap.
fn assert_branch_labels_resolve(function: &CodeFunction) {
    let defined = function
        .instructions
        .iter()
        .filter(|i| i.op == CodeOp::Label)
        .filter_map(|i| i.get("name"))
        .collect::<std::collections::HashSet<_>>();
    for instruction in &function.instructions {
        if !matches!(
            instruction.op,
            CodeOp::Branch
                | CodeOp::BranchEq
                | CodeOp::BranchNe
                | CodeOp::BranchGe
                | CodeOp::BranchGt
                | CodeOp::BranchLe
                | CodeOp::BranchLt
                | CodeOp::BranchHi
                | CodeOp::BranchLo
                | CodeOp::BranchLs
                | CodeOp::BranchMi
                | CodeOp::BranchVc
                | CodeOp::BranchVs
        ) {
            continue;
        }
        if let Some(target) = instruction.get("target") {
            assert!(
                defined.contains(target.as_str()),
                "thunk '{}' branches to undefined label '{target}'",
                function.symbol
            );
        }
    }
}

/// bug-409: a LINK function returning a CSTRUCT with a `CDouble` field
/// (`RETURN structOut`, ABI return a status `CInt32`) branches to
/// `nan_fail`/`inf_fail` in `marshal_struct_out`, but `needs_float` — which
/// gated the label emission — was true only for a direct `CDouble` ABI
/// return. The labels were never emitted, leaving a dangling branch.
#[test]
fn struct_out_cdouble_field_emits_float_fail_labels() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let cstruct = IrCStruct {
        alias: "lib".to_string(),
        name: "Pair".to_string(),
        maps_to: crate::types::ParameterType::parse("PairRec"),
        fields: vec![
            IrCStructField {
                name: "n".to_string(),
                ctype: crate::types::ParameterType::parse("CInt32"),
            },
            IrCStructField {
                name: "x".to_string(),
                ctype: crate::types::ParameterType::parse("CDouble"),
            },
        ],
    };
    let mut record_fields = HashMap::new();
    record_fields.insert(
        crate::types::ParameterType::named("PairRec"),
        vec![
            ("n".to_string(), crate::types::ParameterType::Integer),
            ("x".to_string(), crate::types::ParameterType::Float),
        ],
    );
    let function = IrLinkFunction {
        alias: "lib".to_string(),
        name: "make_pair".to_string(),
        library: "demo".to_string(),
        symbol: "demo_make_pair".to_string(),
        params: vec![],
        return_type: crate::types::ParameterType::parse("PairRec"),
        return_resource: false,
        return_state_type: None,
        abi_slots: vec![IrAbiSlot {
            name: "out".to_string(),
            ctype: crate::types::ParameterType::parse("Pair"),
            direction: crate::ir::AbiDirection::Out,
        }],
        abi_return_name: "status".to_string(),
        abi_return_ctype: crate::types::ParameterType::parse("CInt32"),
        consts: vec![],
        bind_in: vec![],
        bind_state: None,
        bind_state_resource: None,
        success_on: None,
        result: Some(IrLinkExpr::Var("out".to_string())),
        free: None,
        buffers: vec![],
        result_length: None,
    };
    let lowered = lower_link_thunk(
        &function,
        std::slice::from_ref(&cstruct),
        &record_fields,
        TEST_THUNK_CONTEXT,
        &HashSet::new(),
        false,
    )
    .expect("struct-out CDouble thunk must lower");
    assert_branch_labels_resolve(&lowered);
}

/// bug-409: a `CDouble` OUT-slot result (`RETURN dblOut`, ABI return a status
/// `CInt32`) branches to `nan_fail`/`inf_fail` in the OUT-slot marshalling
/// arm (bug-238), but likewise left `needs_float` false — so the labels were
/// never emitted.
#[test]
fn out_slot_cdouble_emits_float_fail_labels() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let function = IrLinkFunction {
        alias: "lib".to_string(),
        name: "read_double".to_string(),
        library: "demo".to_string(),
        symbol: "demo_read_double".to_string(),
        params: vec![],
        return_type: crate::types::ParameterType::parse("Float"),
        return_resource: false,
        return_state_type: None,
        abi_slots: vec![IrAbiSlot {
            name: "out".to_string(),
            ctype: crate::types::ParameterType::parse("CDouble"),
            direction: crate::ir::AbiDirection::Out,
        }],
        abi_return_name: "status".to_string(),
        abi_return_ctype: crate::types::ParameterType::parse("CInt32"),
        consts: vec![],
        bind_in: vec![],
        bind_state: None,
        bind_state_resource: None,
        success_on: None,
        result: Some(IrLinkExpr::Var("out".to_string())),
        free: None,
        buffers: vec![],
        result_length: None,
    };
    let lowered = lower_link_thunk(
        &function,
        &[],
        &HashMap::new(),
        TEST_THUNK_CONTEXT,
        &HashSet::new(),
        false,
    )
    .expect("OUT-slot CDouble thunk must lower");
    assert_branch_labels_resolve(&lowered);
}

/// plan-59-A Phase 3: a native `LINK` resource must never reach the
/// buffer-free path in `emit_resource_block_reclaim`.
///
/// Since plan-59-A every native resource is a resource record, so it now flows
/// through the same drop-reclamation as a built-in `File`. Words 32..80 of a
/// native record are zeroed by the thunk and are NOT buffer pointers; handing
/// them to `arena_free` would free addresses the resource never owned.
///
/// What keeps that from happening is `resource_uses_io_buffers`, which is a
/// bare name comparison against `"fs.File"`. That makes the guarantee POSITIONAL
/// — true only because no other type is spelled `File` — and plan-59-A's Open
/// Decision required it be pinned by a test rather than asserted, precisely
/// because a positional fact is the kind that drifts.
///
/// This test fails the moment a second type is given I/O buffers, which is the
/// signal to re-check whether native records can reach that path.
#[test]
fn only_the_builtin_file_resource_uses_io_buffers() {
    // The one type that owns the two fixed-capacity I/O buffers.
    assert!(CodeBuilder::resource_uses_io_buffers(
        &crate::types::ParameterType::declared("fs.File")
    ));
    // A `STATE`-carrying spelling is still the same base type.
    assert!(CodeBuilder::resource_uses_io_buffers(
        &crate::types::ParameterType::declared("fs.File STATE Cursor")
    ));

    // Every resource type a native `LINK` block declares in-tree, plus the
    // other built-in resources. None may take the buffer-free path.
    for type_ in [
        "Db",
        "Stmt",
        "SoundFile",
        "SoundFile STATE FileInfo",
        "tcp.Socket",
        "tcp.Listener",
        "udp.Socket",
        "audio.AudioInput",
        "audio.AudioOutput",
    ] {
        assert!(
            !CodeBuilder::resource_uses_io_buffers(&crate::types::ParameterType::declared(type_)),
            "{type_} must not take the I/O-buffer free path: its record's \
                 words 32..80 are not buffer pointers"
        );
    }
}

/// bug-296: a LINK thunk calls a real C function, so its arguments follow the
/// target's EXTERNAL C ABI. SysV x86-64 passes six integer arguments in
/// registers; the backend's `CALL_ARGS` extends the list with rax/rbp for
/// arguments 7 and 8, which is sound for the compiler's own calls but hands an
/// external callee two registers it never reads -- it takes those from the
/// stack, so a >=7-integer-slot native function was called with garbage
/// trailing arguments, silently and with no diagnostic. aarch64 and riscv64
/// have 8 real argument registers and are unaffected.
#[test]
fn seven_integer_slots_are_rejected_on_x86_and_accepted_on_aarch64() {
    let seven_int_slots = |count: usize| IrLinkFunction {
        alias: "lib".to_string(),
        name: "seven".to_string(),
        library: "demo".to_string(),
        symbol: "demo_seven".to_string(),
        // Each ABI slot is sourced from a wrapper parameter of the same name.
        params: (0..count)
            .map(|i| (format!("a{i}"), crate::types::ParameterType::Integer))
            .collect(),
        return_type: crate::types::ParameterType::parse("Integer"),
        return_resource: false,
        return_state_type: None,
        abi_slots: (0..count)
            .map(|i| IrAbiSlot {
                name: format!("a{i}"),
                ctype: crate::types::ParameterType::parse("CInt64"),
                direction: crate::ir::AbiDirection::In,
            })
            .collect(),
        abi_return_name: "return".to_string(),
        abi_return_ctype: crate::types::ParameterType::parse("CInt64"),
        consts: vec![],
        bind_in: vec![],
        bind_state: None,
        bind_state_resource: None,
        success_on: None,
        result: None,
        free: None,
        buffers: vec![],
        result_length: None,
    };
    let lower = |count: usize| {
        lower_link_thunk(
            &seven_int_slots(count),
            &[],
            &HashMap::new(),
            TEST_THUNK_CONTEXT,
            &HashSet::new(),
            false,
        )
    };

    // x86-64: six is the SysV limit, so six lowers and seven is refused rather
    // than emitting a call that passes arguments the callee never reads.
    mir::set_backend(&crate::arch::x86_64::backend::X86_64_BACKEND);
    assert!(
        lower(6).is_ok(),
        "six integer slots must still lower on x86"
    );
    let err = match lower(7) {
        Ok(_) => panic!("seven integer slots must be refused on x86"),
        Err(err) => err,
    };
    assert!(
        err.contains("integer ABI slots") && err.contains("seven"),
        "unexpected error: {err}"
    );

    // aarch64 has eight real argument registers, so the same function is fine.
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    assert!(
        lower(7).is_ok(),
        "seven integer slots are within AAPCS64's eight argument registers"
    );
    assert!(lower(8).is_ok(), "eight is AAPCS64's limit");
}

/// The `maxBuffer` ceiling these tests lower against: the project.json
/// default of 64 MiB. Named rather than inlined so a test asserting the gate
/// reads against the same number the default produces.
const TEST_THUNK_CONTEXT: ThunkContext = ThunkContext {
    index: 0,
    globals_base: 0,
    free_slot: None,
    // The project.json `options.maxBuffer` default, so a test asserting the size gate
    // reads against the same number a project with no `options.maxBuffer` gets.
    max_buffer_bytes: 64 * 1024 * 1024,
};

/// Every ctype the allow-list accepts must reach a real marshaling arm.
///
/// plan-50-A closed the slot-ctype namespace and turned `emit_return_passthrough`'s
/// default arm into an `Err`. The allow-list is hand-written (Rust cannot
/// enumerate match arms), so this walks every accepted name through
/// `lower_link_thunk` and asserts none of them lands on that arm. Without it,
/// adding a ctype to the list without a marshaling arm would only fail at the
/// first binding that used it.
#[test]
fn every_known_ctype_lowers() {
    // Lowering reads the active backend for register/instruction shapes; the
    // ctype set under test is backend-independent, so any backend serves.
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);

    // The full accepted set, taken FROM the vocabulary rather than copied
    // beside it (plan-113). `ir::link::tests::ctype_list_is_exhaustive` used
    // to hold a hand-written array in sync with `abi_slot_ctype_is_known`;
    // seeding from `CAbiType::ALL` removes both the array and the drift it
    // could suffer. What this test still proves — and what an exhaustive
    // `match` does NOT — is that each arm actually LOWERS.
    const CTYPES: [CAbiType; 16] = CAbiType::ALL;

    // `CBuffer` is covered by neither loop below, because it fits neither
    // shape: it is invalid as the ABI return (loop 1) and invalid as an IN
    // slot (loop 2). Its only legal position is an OUT slot with a `BUFFER …
    // SIZE` clause, so it gets its own case at the end of this test.
    //
    // plan-58-A shipped that case as an assertion that CBuffer must FAIL to
    // lower, deliberately rigged to break the moment plan-58-B landed the
    // marshaler — which is exactly what happened, rather than the ctype
    // quietly staying uncovered behind a silent filter.
    const OWN_SHAPE_ONLY: &[CAbiType] = &[CAbiType::Buffer];

    // Every ctype valid as an ABI *return* must reach a return arm. This is the
    // arm that can `Err`, and it is how the guard caught `CString` having no
    // return meaning at all (a `char *` return is `CPtr` + a `String` wrapper).
    for ctype in CTYPES.iter().filter(|c| {
        abi_ctype_valid_as_return(&crate::types::ParameterType::C(**c))
            && !OWN_SHAPE_ONLY.contains(c)
    }) {
        let returns_value = *ctype != CAbiType::Void;
        let function = IrLinkFunction {
            alias: "lib".to_string(),
            name: format!("ret_{ctype}"),
            library: "demo".to_string(),
            symbol: "demo_f".to_string(),
            params: vec![],
            // A `CPtr` return with an `Integer` wrapper takes the raw path; the
            // `String` copy-out path is covered by the sqlite3 runtime tests.
            return_type: if returns_value {
                crate::types::ParameterType::parse("Integer")
            } else {
                crate::types::ParameterType::parse("Nothing")
            },
            return_resource: false,
            return_state_type: None,
            abi_slots: vec![],
            abi_return_name: if returns_value { "return" } else { "status" }.to_string(),
            abi_return_ctype: crate::types::ParameterType::C(*ctype),
            consts: vec![],
            bind_in: vec![],
            bind_state: None,
            bind_state_resource: None,
            success_on: None,
            result: None,
            free: None,
            buffers: vec![],
            result_length: None,
        };
        let lowered = lower_link_thunk(
            &function,
            &[],
            &HashMap::new(),
            TEST_THUNK_CONTEXT,
            &HashSet::new(),
            false,
        );
        assert!(
            lowered.is_ok(),
            "accepted return ctype {ctype} does not lower: {:?}",
            lowered.err()
        );
    }

    // Every ctype valid as an ABI *argument* must stage without error. Pinned
    // with CONST so the slot needs no wrapper parameter.
    for ctype in CTYPES
        .iter()
        .filter(|c| abi_ctype_valid_as_argument(&crate::types::ParameterType::C(**c)))
    {
        let function = IrLinkFunction {
            alias: "lib".to_string(),
            name: format!("arg_{ctype}"),
            library: "demo".to_string(),
            symbol: "demo_f".to_string(),
            params: vec![],
            return_type: crate::types::ParameterType::parse("Nothing"),
            return_resource: false,
            return_state_type: None,
            abi_slots: vec![IrAbiSlot {
                name: "pinned".to_string(),
                ctype: crate::types::ParameterType::C(*ctype),
                direction: crate::ir::AbiDirection::In,
            }],
            abi_return_name: "status".to_string(),
            abi_return_ctype: crate::types::ParameterType::parse("CInt32"),
            consts: vec![("pinned".to_string(), 0)],
            bind_in: vec![],
            bind_state: None,
            bind_state_resource: None,
            success_on: None,
            result: None,
            free: None,
            buffers: vec![],
            result_length: None,
        };
        let lowered = lower_link_thunk(
            &function,
            &[],
            &HashMap::new(),
            TEST_THUNK_CONTEXT,
            &HashSet::new(),
            false,
        );
        assert!(
            lowered.is_ok(),
            "accepted argument ctype {ctype} does not lower: {:?}",
            lowered.err()
        );
    }

    // plan-58-B: a well-formed `OUT CBuffer` — one that passes every
    // `check_buffer_slots` rule — must LOWER. The runtime proof that the bytes
    // are right lives in `tests/rt-behavior/native/native-cbuffer-read-rt`;
    // this is the drift guard that keeps the ctype covered at all.
    for ctype in OWN_SHAPE_ONLY {
        let function = IrLinkFunction {
            alias: "lib".to_string(),
            name: format!("out_{ctype}"),
            library: "demo".to_string(),
            symbol: "demo_f".to_string(),
            params: vec![(
                "n".to_string(),
                crate::types::ParameterType::parse("Integer"),
            )],
            return_type: crate::types::ParameterType::parse(crate::ir::BYTE_LIST_TYPE),
            return_resource: false,
            return_state_type: None,
            abi_slots: vec![IrAbiSlot {
                name: "buf".to_string(),
                ctype: crate::types::ParameterType::C(*ctype),
                direction: crate::ir::AbiDirection::Out,
            }],
            abi_return_name: "status".to_string(),
            abi_return_ctype: crate::types::ParameterType::parse("CInt32"),
            consts: vec![],
            bind_in: vec![],
            bind_state: None,
            bind_state_resource: None,
            success_on: None,
            result: Some(crate::ir::IrLinkExpr::Var("buf".to_string())),
            free: None,
            buffers: vec![crate::ir::IrBuffer {
                slot: "buf".to_string(),
                size: crate::ir::IrLinkExpr::Var("n".to_string()),
            }],
            result_length: Some(crate::ir::IrLinkExpr::Var("status".to_string())),
        };
        let lowered = lower_link_thunk(
            &function,
            &[],
            &HashMap::new(),
            TEST_THUNK_CONTEXT,
            &HashSet::new(),
            false,
        );
        assert!(
            lowered.is_ok(),
            "accepted OUT-slot ctype {ctype} does not lower: {:?}",
            lowered.err()
        );
    }
}

/// bug-431: build a `LinkLibraries` with one logical library resolved to a
/// single vendored locator, so `emit_link_support` can lower a Windows
/// initializer that must load it from the exe-relative `vendor/` directory.
fn one_vendored_library(logical: &str, dlopen_name: &str) -> LinkLibraries {
    use crate::binary_repr::NativeLibraryLocator;
    use crate::manifest::libraries::LibType;
    let mut resolved = HashMap::new();
    resolved.insert(
        logical.to_string(),
        ResolvedLibrary {
            dlopen_name: dlopen_name.to_string(),
            declaring_unit: "demo".to_string(),
            locator: NativeLibraryLocator {
                os: "windows".to_string(),
                arch: Some("x86_64".to_string()),
                libc: None,
                lib_type: LibType::Vendor,
                source: dlopen_name.to_string(),
                hash: Some([0u8; 32]),
            },
        },
    );
    LinkLibraries::from_resolved(resolved)
}

/// The names a `bl` (`CodeOp::BranchLink`) in `function` calls.
fn call_targets(function: &CodeFunction) -> std::collections::HashSet<String> {
    function
        .instructions
        .iter()
        .filter(|i| i.op == CodeOp::BranchLink)
        .filter_map(|i| i.get("target").map(|t| t.to_string()))
        .collect()
}

/// bug-431: a native `LINK` function is a real C call through `blr`, so its
/// integer/pointer result returns in the C-ABI return register (`rax` on
/// x86-64, `x0`/`a0` elsewhere), NOT the aligned MFB result bank. On x86-64
/// those differ (`rdi`/SysV, `rcx`/Win64), so capturing `return_register()`
/// read a clobbered caller-saved register and every native return surfaced as
/// garbage — invisible because x86-64 LINK *execution* is not in the
/// acceptance oracle (macOS aarch64, where the banks coincide, is). The
/// capture must name `c_return(0)`. Asserted at the neutral operand level, so
/// it is backend-independent: the two banks are distinct ABI tokens.
#[test]
fn native_call_result_is_captured_from_c_return_register() {
    mir::set_backend(&crate::arch::x86_64::backend::X86_64_BACKEND);
    let function = IrLinkFunction {
        alias: "lib".to_string(),
        name: "answer".to_string(),
        library: "demo".to_string(),
        symbol: "demo_answer".to_string(),
        params: vec![(
            "n".to_string(),
            crate::types::ParameterType::parse("Integer"),
        )],
        return_type: crate::types::ParameterType::parse("Integer"),
        return_resource: false,
        return_state_type: None,
        abi_slots: vec![IrAbiSlot {
            name: "n".to_string(),
            ctype: crate::types::ParameterType::parse("CInt64"),
            direction: crate::ir::AbiDirection::In,
        }],
        abi_return_name: "return".to_string(),
        abi_return_ctype: crate::types::ParameterType::parse("CInt64"),
        consts: vec![],
        bind_in: vec![],
        bind_state: None,
        bind_state_resource: None,
        success_on: None,
        result: None,
        free: None,
        buffers: vec![],
        result_length: None,
    };
    let lowered = lower_link_thunk(
        &function,
        &[],
        &HashMap::new(),
        TEST_THUNK_CONTEXT,
        &HashSet::new(),
        false,
    )
    .expect("thunk must lower");
    let blr = lowered
        .instructions
        .iter()
        .position(|i| i.op == CodeOp::BranchLinkRegister)
        .expect("a LINK thunk calls the native function through blr");
    let capture = &lowered.instructions[blr + 1];
    let want = abi::store_u64(abi::c_return(0), abi::stack_pointer(), 0);
    let wrong = abi::store_u64(abi::return_register(), abi::stack_pointer(), 0);
    assert_eq!(
        capture.get("src"),
        want.get("src"),
        "the native call's result must be captured from the C-return register (rax on \
             x86-64), got src {:?}",
        capture.get("src")
    );
    assert_ne!(
        capture.get("src"),
        wrong.get("src"),
        "must not capture the aligned MFB result bank (rdi/SysV, rcx/Win64 on x86-64)"
    );
}

/// bug-431: a `LINK`-using module lowered for `windows-x86_64` must build its
/// load-time initializer on the Win32 loader (`LoadLibraryExA` +
/// `GetProcAddress`), NOT the POSIX `dlopen`/`dlsym` pair — Windows has no
/// `dlopen`, so the initializer that emits it cannot even bind. A vendored
/// library is loaded from the exe-relative `vendor/` directory, so the path is
/// built with `GetModuleFileNameA` + `PathRemoveFileSpecA` + `lstrcatA`.
#[test]
fn windows_link_initializer_uses_win32_loader_not_dlopen() {
    mir::set_backend(&crate::arch::x86_64::backend::WIN64_BACKEND);
    let function = IrLinkFunction {
        alias: "lib".to_string(),
        name: "answer".to_string(),
        library: "demo".to_string(),
        symbol: "demo_answer".to_string(),
        params: vec![],
        return_type: crate::types::ParameterType::parse("Integer"),
        return_resource: false,
        return_state_type: None,
        abi_slots: vec![],
        abi_return_name: "return".to_string(),
        abi_return_ctype: crate::types::ParameterType::parse("CInt64"),
        consts: vec![],
        bind_in: vec![],
        bind_state: None,
        bind_state_resource: None,
        success_on: None,
        result: None,
        free: None,
        buffers: vec![],
        result_length: None,
    };
    // Provide every symbol either code path can reference (the current
    // `dlopen`/`dlsym` default AND the intended Win32 loader) so the lowering
    // reaches its emit rather than erroring on a missing import — the test
    // then asserts on WHICH loader was emitted.
    let platform_imports: HashMap<String, String> = [
        ("dlopen", "kernel32.dll"),
        ("dlsym", "kernel32.dll"),
        ("LoadLibraryExA", "kernel32.dll"),
        ("GetProcAddress", "kernel32.dll"),
        ("GetModuleFileNameA", "kernel32.dll"),
        ("lstrcatA", "kernel32.dll"),
        ("PathRemoveFileSpecA", "shlwapi.dll"),
    ]
    .iter()
    .map(|(s, l)| (s.to_string(), l.to_string()))
    .collect();
    let libraries = one_vendored_library("demo", "demo-libanswer.dll");
    let support = emit_link_support(
        std::slice::from_ref(&function),
        &[],
        &HashMap::new(),
        LinkCodegenOptions {
            globals_base: 0,
            max_buffer_bytes: 64 * 1024 * 1024,
        },
        &platform_imports,
        &crate::target::win_x86_64::code::Platform,
        &libraries,
        &HashSet::new(),
    )
    .expect("Windows LINK support must lower");
    let initializer = &support.functions[0];
    let calls = call_targets(initializer);
    assert!(
        calls.contains("LoadLibraryExA"),
        "Windows _mfb_linker_init must load libraries via LoadLibraryExA, got calls: {calls:?}"
    );
    assert!(
        calls.contains("GetProcAddress"),
        "Windows _mfb_linker_init must resolve symbols via GetProcAddress, got calls: {calls:?}"
    );
    assert!(
        !calls.contains("dlopen") && !calls.contains("dlsym"),
        "Windows _mfb_linker_init must not call the POSIX dlopen/dlsym (unbindable on \
             Windows), got calls: {calls:?}"
    );
    // A vendored DLL is resolved from the exe-relative `vendor/` directory:
    // the path is built at load time rather than trusting the default search.
    assert!(
        calls.contains("GetModuleFileNameA"),
        "a vendored Windows LINK library must resolve the exe directory via \
             GetModuleFileNameA, got calls: {calls:?}"
    );
}
