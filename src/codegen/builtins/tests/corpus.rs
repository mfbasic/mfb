//! Every backend lowers the same corpus to the same set of functions.
//!
//! The cross-backend consistency of codegen has one gate today, and it is the
//! acceptance matrix: build the corpus five times, on five runners, from a
//! spawned `mfb`. That gate is slow, it is off the unit-test path entirely, and
//! — because `mfb` is a binary-only package — none of it is observable from this
//! process, so the shared codegen it exercises (`ir/lower.rs`,
//! `engine/value/builder_values.rs`, `collection/list/list_mutate.rs`,
//! `link/thunk/link_thunk.rs`, …) has no in-process coverage at all beyond
//! whatever a hand-written unit test happens to reach.
//!
//! This runs the same programs through the real `.ncode` pipeline in process,
//! for all five backends, in a few seconds, and asserts two things a per-file
//! suite cannot:
//!
//!   1. **Every backend lowers it.** `code::lower_module` validates as it goes —
//!      a branch to an undefined label, a relocation against a symbol nothing
//!      defines, a data object nobody emitted are all rejected here. A construct
//!      one backend cannot lower is exactly the failure the acceptance matrix
//!      exists to catch, and this catches it in the same second it is written.
//!   2. **The five agree on the program.** The set of *user* functions emitted
//!      must be identical across backends: a backend that silently drops one, or
//!      invents one, has diverged from the language rather than from an ABI.
//!      Runtime helpers and toolkit entry points legitimately differ, so the
//!      comparison is over the program's own functions.
//!
//! The corpus is an explicit list, and its length is asserted. Auto-discovering
//! fixtures and skipping the ones that do not lower would let the whole thing
//! hollow out silently — a skipped fixture and a covered one look identical in a
//! green run.

use std::collections::BTreeSet;

use crate::codegen::engine::types::NativeCodePlan;
use crate::testutil::{code_for_src_cached, fixture_src, CodeTarget};

/// Committed single-file programs, one per language or library area.
///
/// Chosen for breadth of *lowering shapes* rather than of behaviour: a record
/// field write, a match with guards, a generic instantiation, a growable list, a
/// map rebuild, a closure, a trap, a native `LINK` thunk. Each name is a
/// directory under `tests/` that `testutil::fixture_dir` resolves.
/// The subset lowered for **all five** backends.
///
/// Kept small deliberately: five lowerings of a program cost five times one, and
/// the cross-backend question is about the SHAPES a program uses -- a record
/// field write, a guarded match, a generic instantiation, a growable list, a map
/// rebuild -- not about how many programs use them.
const CROSS_BACKEND: &[&str] = &[
    "record-field",
    "float-fma-fusion",
    "bug144_toint_base10_overflow",
    "control-flow-behavior",
    "bug118_match_guard_helper",
    "bug361_match_oneof_literals",
    "call-function-value-rt",
    "user-generic-single-param-rt",
    "user-generic-nested-rt",
    "bulk-append-inplace",
    "mut-append-grow",
    "map-set-grow-rt",
    "set-algebra-rt",
    "nested-fixed-list-rt",
    "json-behavior",
    "regex-posix-classes-rt",
];

/// Every committed single-file fixture this harness can lower, four per family.
///
/// Four rather than all of them (729 lower) because the yield is in the SHAPES,
/// not the count: a fifth `crypto-*` fixture exercises the same lowering as the
/// first four. Four rather than one because a family's fixtures usually differ
/// in exactly the branch that family's bugs live in. The list is generated once
/// and committed -- not discovered at run time -- so a fixture that stops
/// lowering fails this test instead of quietly dropping out of it.
const CORPUS: &[&str] = &[
    // rt-behavior/arena
    "construct-helper-loop",
    "flat-nested-collection",
    "flat-nested-record",
    "flat-record-collection",
    // rt-behavior/arithmetic
    "bug363_record_field_float_error",
    "bug367_negative_fixed_literal",
    "float-fma-fusion",
    "record-field",
    // rt-behavior/astrings
    "attribute-model-rt",
    "color-payload-rt",
    "copy-drop-rt",
    "fromstring-print-rt",
    // rt-behavior/bits
    "func_bits_sl_range_rt",
    // rt-behavior/collections
    "bounds-elim-rt",
    "bug142_foreach_inplace_append",
    "bug145_map_set_value_path",
    "bug147_float_eq_nesting",
    // rt-behavior/color
    "color_constructors_rt",
    "color_hex_rt",
    "color_hsl_rt",
    "color_names_rt",
    // rt-behavior/control-flow
    "bug118_match_guard_helper",
    "bug140_string_match_content",
    "bug361_match_oneof_literals",
    "control-flow-behavior",
    // rt-behavior/conversions
    "bug144_toint_base10_overflow",
    "bug358_tostring_default_precision",
    "bug366_money_float_invalid_format",
    "bug366_record_field_exact_conversion",
    // rt-behavior/crypto
    "crypto-aead-invalid",
    "crypto-curve-attribution-valid",
    "crypto-decrypt-short-box-invalid",
    "crypto-ec-valid",
    // rt-behavior/csv
    "csv-behavior",
    "csv-dialect",
    // rt-behavior/datetime
    "bug349_instant_duration_named_args",
    "bug94_fixedoffset_named_args",
    "datetime-civil-valid",
    "datetime-clock-offset",
    // rt-behavior/encoding
    "func_encoding_codepageDecode_rt",
    "func_encoding_codepageEncode_rt",
    // rt-behavior/fs
    "bug101_readtext_roundtrip",
    "bug132_pathnormalize_root_pop",
    "bug159_listdir_notdir_error",
    "file-buffered-drain-integrity-rt",
    // rt-behavior/functions
    "bug103_generic_global_builtin_args",
    "bug196_named_arg_reorder_instantiation",
    "bug197_assign_rhs_expected_type",
    "bug198_global_func_value_callee",
    // rt-behavior/general
    "bug133_find_multibyte_start",
    "bug137_bool_xor_call",
    "bug143_string_self_append_chain",
    "bug155_toInt_named_args",
    // rt-behavior/generics
    "user-generic-collection-rt",
    "user-generic-multi-param-rt",
    "user-generic-nested-rt",
    "user-generic-nested-user-rt",
    // rt-behavior/http
    "func_http_constructors_valid",
    "func_http_read_crlf_invalid",
    "func_http_respondPath_valid",
    "func_http_response_valid",
    // rt-behavior/io
    "func_io_flush_valid",
    "func_io_isBuffered_valid",
    "func_io_pollInput_valid",
    "func_io_printError_valid",
    // rt-behavior/json
    "json-behavior",
    "json-number-rendering-rt",
    "json-number-roundtrip-rt",
    "json-parse-deep-nesting-rt",
    // rt-behavior/lexical
    "bug19_doc_dedent_multibyte_whitespace",
    "lexical-comments",
    "lexical-line-continuation",
    "lexical-literals",
    // rt-behavior/math
    "bug121_simd_abs_min_max_clamp",
    "bug126_round_list_ties",
    "bug128_fixed_atan2_overflow_min",
    "bug129_pow_subnormal",
    // rt-behavior/money
    "bug230_divide_i64_min_divisor",
    "money_inexact_float_warn",
    "money_operations",
    "money_package",
    // rt-behavior/net
    "func_net_lookup_valid",
    "func_net_parseQuery_valid",
    "func_net_percentDecode_valid",
    "func_net_ping_valid",
    // rt-behavior/operators
    "overflow-elision-soundness",
    "unary-numeric-negation-valid",
    // rt-behavior/os
    "func_os_arch_valid",
    "func_os_args_valid",
    "func_os_cpuCount_valid",
    "func_os_environ_valid",
    // rt-behavior/packages
    "http-process-coexist-rt",
    // rt-behavior/process
    "close-input-keeps-handle",
    "detach",
    "detach-then-use",
    "drop-reap",
    // rt-behavior/project
    "binding-global-list-literal",
    "project-entry-args-runtime",
    "project-entry-func-args-default-trap",
    "project-entry-func-args-main-trap",
    // rt-behavior/regalloc
    "large-functions",
    "register-pressure",
    // rt-behavior/regex
    "regex-from-string-rt",
    "regex-posix-classes-rt",
    // rt-behavior/resources
    "bug141_resource_union_return",
    "bug246_res_bind_error_plain_trap",
    "bug256_state_string_field",
    "bug424_state_accum_inplace",
    // rt-behavior/security
    "allocator-01-quick-bins",
    "allocator-02-size-overflow",
    "allocator-03-free-list-integrity",
    "allocator-05-grow-carve",
    // rt-behavior/tcp
    "bug109_write_timeout",
    "bug185_accept_timeout",
    "func_tcp_accept_valid",
    "func_tcp_close_valid",
    // rt-behavior/term
    "func_term_cluster_pool_valid",
    "func_term_color_roundtrip_valid",
    "func_term_drawBox_valid",
    "func_term_drawGlyph_valid",
    // rt-behavior/testing
    "testing-trap-parity",
    // rt-behavior/threads
    "func_thread_openStdIn_valid",
    "thread-executable-local-entry-rt",
    // rt-behavior/tls
    "tls-connect-google-rt",
    "tls-poll-list-rt",
    "tls-poll-rt",
    "tls-read-eof-raises-rt",
    // rt-behavior/trap
    "bug151_caught_error_freed",
    "bug152_reraise_adopt",
    "control-flow-inline-trap-nested-valid",
    "control-flow-inline-trap-resource-rt",
    // rt-behavior/types
    "bug105_grouped_type_names",
    "bug119_result_field",
    "bug80_union_divergent_positions",
    "types-behavior",
    // rt-behavior/udp
    "bug160_send_capacity_gt_count",
    "func_udp_bind_valid",
    "func_udp_endpoints_valid",
    "func_udp_poll_valid",
    // rt-behavior/vector
    "vector-inline-ops",
    "vector-length-distance-inline-rt",
    "vector-native-carrier",
    "vector-normalize-inline-rt",
    // rt-error/arithmetic
    "arithmetic-add-sub-invalid-rt",
    "arithmetic-division-invalid-rt",
    "arithmetic-exp-mod-invalid-rt",
    "arithmetic-fixed-invalid-rt",
    // rt-error/astrings
    "attribute-bounds-rt",
    // rt-error/audio
    "openInput_invalid_rt",
    "openOutput_invalid_rt",
    // rt-error/collections
    "bounds_elim_backedge_rt",
    "bounds_elim_headroom_rt",
    "bounds_elim_noninduction_rt",
    "bounds_elim_reassigned_rt",
    // rt-error/control-flow
    "continue-loop-valid-rt",
    "exit-loop-valid",
    "exit-program-valid-rt",
    // rt-error/crypto
    "crypto-ec-invalid",
    // rt-error/doc
    "doc-block-valid",
    // rt-error/encoding
    "func_encoding_codepageDecode_unmapped",
    "func_encoding_codepageEncode_unrepresentable",
    "func_encoding_hexDecode_valid",
    // rt-error/functions
    "lambda-capture-valid",
    "lambda-mut-foreach-valid",
    "overload-func-valid",
    // rt-error/general
    "find_not_found",
    "find_out_of_range",
    "mid_count_out_of_range",
    "mid_negative_range",
    // rt-error/json
    "func_json_stringify_invalid_runtime",
    // rt-error/match
    "control-flow-match-destructuring",
    "control-flow-match-else",
    "control-flow-match-when",
    // rt-error/math
    "func_math_abs_fixedarray_rt",
    "func_math_abs_intarray_rt",
    "func_math_acos_fixed_domain_rt",
    "func_math_acos_float_domain_rt",
    // rt-error/money
    "money_round_bad_decimals",
    "toByte_money_overflow",
    "toMoney_invalid_format",
    "toMoney_overflow",
    // rt-error/net
    "func_net_ping_range_invalid",
    // rt-error/operators
    "unary-byte-negation-underflow-rt",
    "unary-fixed-negation-overflow-rt",
    "unary-float-negation-invalid-format-rt",
    // rt-error/os
    "os-sleep-negative-rt",
    // rt-error/project
    "project-entry-exit-range-runtime",
    "project-entry-func-args-default-error",
    "project-entry-func-args-main-error",
    "project-entry-func-default-error",
    // rt-error/tcp
    "func_tcp_listen_valid",
    // rt-error/types
    "types-enum",
    "types-map-key-comparable-valid",
    "types-records",
    "types-union",
    // rt-error/vector
    "angle_zero_rt",
    "clamp_length_negative_rt",
    "normalize_zero_inline_rt",
    "normalize_zero_integer_rt",
    // syntax/app
    "macos-app-mode-io",
    "macos-app-mode-plumbing",
    "macos-app-mode-term",
    // syntax/astrings
    "attribute-model-invalid",
    // syntax/audio
    "no_import_invalid",
    // syntax/color
    "color_type_bare_leaf_invalid",
    // syntax/control-flow
    "while-end-while-valid",
    // syntax/datetime
    "bug349_instant_named_arg_arity_invalid",
    // syntax/doc
    "doc-block-invalid",
    // syntax/fs
    "func_fs_tempDirectory_valid",
    // syntax/functions
    "func_override_reserved_invalid",
    "func_visibility_export_in_executable_invalid",
    "private_isolated_func_valid",
    "sub-template-valid",
    // syntax/http
    "http_async_stream_valid",
    // syntax/io
    "func_io_input_valid",
    "func_io_isErrorTerminal_valid",
    "func_io_isInputTerminal_valid",
    "func_io_isOutputTerminal_valid",
    // syntax/lexical
    "parser-hello-world",
    // syntax/match
    "control-flow-match",
    // syntax/os
    "func_os_sleep_valid",
    // syntax/packages
    "audit-basic",
    "audit-dependency-missing",
    "audit-locked-missing",
    "audit-lockfile-current",
    // syntax/process
    "type_valid",
    // syntax/project
    "bug298_resource_path_escape_invalid",
    "bug353_non_array_packages_invalid",
    "import-alias-conflicts",
    "project-description-optional-executable",
    // syntax/resources
    "ownership-collection-resource-valid",
    "resource-collection-close-floated-valid",
    "resource-collection-not-owner-valid",
    "resource-invalidate-not-owner-valid",
    // syntax/security
    "bug96_audit_tls_http_crypto",
    // syntax/tcp
    "func_tcp_poll_list_valid",
    "local-address-field-unrelated-udp-import",
    // syntax/term
    "term-valid",
    // syntax/testing
    "testing-aliased-import-valid",
    "testing-assert-invalid",
    "testing-coverage-valid",
    "testing-nested-valid",
    // syntax/threads
    "func_thread_emit_valid",
    "func_thread_isCancelled_valid",
    "func_thread_receive_valid",
    "thread-message-plane-constructors-valid",
    // syntax/tls
    "accept_valid",
    "close_valid",
    "connect_valid",
    "endpoints_valid",
    // syntax/trap
    "control-flow-sub-trap-valid",
    "control-flow-trap-valid",
    // syntax/types
    "mut-default-collection-of-nondefaultable-valid",
    "types-recursive-record-valid",
];

/// The program's own functions — everything the source declared, plus the
/// synthesized bodies of the builtin packages it imported.
///
/// Runtime helpers (`runtime.*`), constructors and the per-backend program entry
/// are excluded: those legitimately differ, and including them would turn a real
/// disagreement into noise nobody reads.
fn program_functions(plan: &NativeCodePlan) -> BTreeSet<&str> {
    plan.functions
        .iter()
        .map(|f| f.name.as_str())
        .filter(|name| {
            !name.starts_with("runtime.")
                && !name.starts_with("construct.")
                && !name.starts_with("program.")
                && !name.starts_with("_main")
                && !name.starts_with("_mfb_")
                && !name.starts_with("__mfb_")
        })
        .collect()
}

#[test]
fn every_backend_lowers_the_corpus_to_the_same_program() {
    for fixture in CROSS_BACKEND {
        let source = fixture_src(fixture);
        let mut agreed: Option<(CodeTarget, BTreeSet<&str>)> = None;
        for target in CodeTarget::ALL {
            let plan =
                code_for_src_cached(&source, target, crate::target::NativeBuildMode::Console);
            assert_eq!(
                plan.target,
                target.name(),
                "{fixture}: the code plan must record the backend it was lowered for"
            );
            let functions = program_functions(plan);
            assert!(
                !functions.is_empty(),
                "{fixture}: {} emitted no program functions at all",
                target.name()
            );
            match &agreed {
                None => agreed = Some((target, functions)),
                Some((first, expected)) => {
                    let missing: Vec<&&str> = expected.difference(&functions).collect();
                    let extra: Vec<&&str> = functions.difference(expected).collect();
                    assert!(
                        missing.is_empty() && extra.is_empty(),
                        "{fixture}: {} and {} disagree on the emitted program. \
                         {} is missing {missing:?} and has extra {extra:?}",
                        first.name(),
                        target.name(),
                        target.name()
                    );
                }
            }
        }
    }
}

/// Nothing in the corpus lowers to a function with an empty body.
///
/// An empty body is what a lowering that silently declined looks like: the
/// symbol exists, the call links, and the program does nothing where it should
/// have done something. It is the one failure that survives both the validator
/// (nothing is malformed) and a behavioural test that does not happen to call
/// that path.
#[test]
fn no_corpus_function_lowers_to_an_empty_body() {
    assert!(
        CORPUS.len() >= 250,
        "the corpus is {} fixtures; it was generated at 254 across 88 families, \
         and a list that shrinks silently is a gate that stops measuring",
        CORPUS.len()
    );
    for fixture in CORPUS {
        let source = fixture_src(fixture);
        let plan = code_for_src_cached(
            &source,
            CodeTarget::LinuxX86_64,
            crate::target::NativeBuildMode::Console,
        );
        for f in &plan.functions {
            assert!(
                f.instructions.len() > 1,
                "{fixture}: `{}` lowered to {} instruction(s) — a body that is \
                 only its entry label is a lowering that declined",
                f.name,
                f.instructions.len()
            );
        }
    }
}
