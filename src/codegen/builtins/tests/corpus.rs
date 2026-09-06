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
use crate::testutil::{fixture_src, try_code_for_src, CodeTarget};

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
    // scalar arithmetic, conversions, control flow
    "record-field",
    "float-fma-fusion",
    "bug144_toint_base10_overflow",
    "control-flow-behavior",
    "bug118_match_guard_helper",
    "bug361_match_oneof_literals",
    // functions, generics, callbacks
    "call-function-value-rt",
    "user-generic-single-param-rt",
    "user-generic-nested-rt",
    // collections: the mutation and rebuild paths
    "bulk-append-inplace",
    "mut-append-grow",
    "map-set-grow-rt",
    "set-algebra-rt",
    "nested-fixed-list-rt",
    "inplace-grow-free",
    "reduce-accumulator-reclaim-rt",
    // strings, json, regex
    "json-behavior",
    "regex-posix-classes-rt",
    // The per-backend surface: a thread spawns a trampoline, a RES type emits a
    // close op and a scope drop, and a trap emits an error route. Each is
    // per-ARCH code that a single-backend lowering cannot reach.
    //
    // Every name here is one this harness can lower from `src/main.mfb` alone.
    // Two fixtures cannot be: `thread-fixed-list-transfer-rt` and
    // `p121d-state-reach-rt` name their worker through an imported package, and
    // a qualified entry resolves through `imported_signatures`, which a
    // single-source project never populates -- so both reported "thread.start
    // entry point must name an ISOLATED FUNC" on all five backends, a missing
    // input wearing the costume of a front-end limitation.
    //
    // They are no longer skipped, only lowered elsewhere:
    // `fixture_projects.rs` runs them through
    // `testutil::try_code_for_fixture_project`, which reads the manifest's
    // `packages` and hands both the signatures and the merged package IR to
    // lowering. Picking a fixture by hand without checking is still how a
    // cross-backend suite ends up reporting a front-end limitation as a backend
    // disagreement.
    "thread-executable-local-entry-rt",
    "thread-start-local-entry-valid",
    "p121d-state-ops-rt",
    "p121d-state-splice-rt",
    "func-bare-trap-loop-leak-rt",
    "return-param-borrow-rt",
    "get-borrow-match-rt",
    "recursive-get-then-grow-rt",
];

/// Every committed single-file fixture this harness can lower, four per family.
///
/// Three entries are hand-added rather than generated, each aimed at a named
/// gap the cap of four had dropped: `func_typename_builtin_calls` (the
/// compile-time `typeName` fold, across every builtin family),
/// `vector-promotion` (a vector held in registers rather than as a block), and
/// `http-tcp-transport-rt` (a resource UNION carrying STATE, whose error-path
/// binding needs the closed-record default).
///
/// Four per family for most, and ALL of them for the nineteen families whose
/// lowering shapes carry the remaining gaps -- collection mutation, the
/// RES/arena transfer paths, generics, traps, threads, and the error corpora.
/// Four is right where the yield is in the SHAPES (a fifth `crypto-*` fixture
/// exercises the same lowering as the first four); it is wrong where a family's
/// fixtures differ in exactly the branch that family's bugs live in. The list is
/// generated once and committed -- not discovered at run time -- so a fixture
/// that stops lowering fails this test instead of quietly dropping out of it.
///
/// 704 of the 1,315 single-file fixtures under `tests/` lower here at all, and
/// the 611 that do not were bucketed by their failure rather than assumed:
/// ~400 are `tests/syntax/**` programs that are SUPPOSED to be rejected (the
/// diagnostics suite covers those), 21 declare a native `LINK` and need a
/// library table (`link.rs` covers those), 21 are deliberate parse errors, and
/// the rest reference an imported `.mfp` package, which this harness has no way
/// to supply. None of it is a product gap. Taking all 704 rather than these 421
/// was measured too: +170 seconds of unit-suite time for 242 lines and one
/// file, so the sampling stops here.
pub(super) const CORPUS: &[&str] = &[
    // rt-behavior/arena
    "construct-helper-loop",
    "flat-nested-collection",
    "flat-nested-record",
    "flat-record-collection",
    "flat-record-string",
    "flat-union",
    "member-iterable-mutate",
    "return-copy-elision",
    "scope-drop-free",
    "scope-drop-free-union",
    "string-concat",
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
    "bug147_set_error_path_leak",
    "bug496_operand_snapshot_rt",
    "builtin-pair-partition-valid",
    "bulk-append-inplace",
    "chunks-string-native-rt",
    "collection-list-bindings",
    "collection-map-bindings",
    "collection-memory-grow-rt",
    "collection-of-function-rt",
    "collection-payload-alignment-rt",
    "collection-set-string-grow-rt",
    "collections-artifact-coverage-rt",
    "findlast-native-rt",
    "flatten-inline-rt",
    "func_map_getor_hash_probe",
    "get-borrow-match-rt",
    "groupby-string-value-native-rt",
    "hof-string-item-lifetime-rt",
    "inplace-grow-free",
    "list-ops-codegen-rt",
    "list-order-invariant-rt",
    "list-payload-order-rt",
    "list-unordered-data",
    "map-removekey-inplace-rt",
    "map-set-grow-rt",
    "merge-native-rt",
    "mut-append-grow",
    "nested-fixed-list-rt",
    "p121b-fixedwidth-splice-order-rt",
    "p121b-removeat-insert-aliasing-rt",
    "p121b-removeat-recursive-union-rt",
    "p121c-record-field-add-grow-rt",
    "p121c-record-field-remove-rt",
    "p121c-record-field-removekey-rt",
    "p121c-record-field-set-rt",
    "p121c-record-field-splice-rt",
    "p121f-string-set-readback-rt",
    "p121g-reduce-accumulator-rt",
    "partition-native-trap-rt",
    "recursive-get-then-grow-rt",
    "reduce-accumulator-reclaim-rt",
    "return-param-borrow-rt",
    "set-algebra-rt",
    "set-behavior-rt",
    "set-inplace-add-rt",
    "sort-string-gather-rt",
    "sortby-string-gather-rt",
    "window-string-native-rt",
    "zip-string-native-rt",
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
    "control-flow-if",
    // rt-behavior/conversions
    "bug144_toint_base10_overflow",
    "bug358_tostring_default_precision",
    "bug366_money_float_invalid_format",
    "bug366_record_field_exact_conversion",
    "bug91_fixed_sub_ulp_literal",
    "tofloat-correct-rounding-corpus-rt",
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
    "bug78_function_ref_static_descriptor",
    "call-function-value-rt",
    "closure-call-register-pressure-rt",
    "closure-scope-drop-rt",
    "func_override_len_user",
    "func_override_no_hijack_valid",
    "func_override_overloaded",
    "func_override_toint_user",
    "func_override_tostring_user",
    "func_return_overload_valid",
    "func_typesystem_error_valid",
    "func_typesystem_result_pattern_valid",
    "function-value-error-propagates-rt",
    "indirect-inline-trap-rt",
    "enum-elements-rt",
    "hof-callback-failure-rt",
    "inline-trap-positions-rt",
    "list-literal-numeric-coercion-rt",
    "comparable-records-rt",
    "byref-capture-rt",
    "inplace-decline-aliasing-rt",
    "collection-compare-payloads-rt",
    "state-scalar-inplace-decline-rt",
    "default-values-rt",
    "overload-sub-valid",
    "user-function-default-args-result-valid",
    "user-function-stack-args-valid",
    // rt-behavior/general
    "func_typename_builtin_calls",
    "bug133_find_multibyte_start",
    "bug137_bool_xor_call",
    "bug143_string_self_append_chain",
    "bug155_toInt_named_args",
    // rt-behavior/generics
    "user-generic-collection-rt",
    "user-generic-multi-param-rt",
    "user-generic-nested-rt",
    "user-generic-nested-user-rt",
    "user-generic-single-param-rt",
    "user-generic-unknown-refine-rt",
    // rt-behavior/http
    "http-tcp-transport-rt",
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
    "bug427_list_union_state_rt",
    "bug429_owned_list_union_drain_rt",
    "closed-default-drop-rt",
    "closed-default-tls-drop-rt",
    "control-flow-resource-fail-runtime",
    "control-flow-resource-normal-runtime",
    "control-flow-resource-propagate-runtime",
    "control-flow-resource-return-runtime",
    "inline-trap-collection-escape-rt",
    "inline-trap-producer-float-rt",
    "native-resource-import-valid",
    "ownership-resource-manual-close-valid",
    "p121d-state-ops-rt",
    "p121d-state-splice-rt",
    "record-res-field-return-rt",
    "record-res-field-rt",
    "record-res-field-state-rt",
    "record-res-field-state-write-rt",
    "res-rebind-alias-runtime",
    "resource-collection-floats-runtime",
    "resource-collection-map-value-runtime",
    "resource-collection-transfer-runtime",
    "resource-exit-path-cleanup-rt",
    "resource-pointer-across-ops-valid",
    "resource-reclaim-loop-valid",
    "resource-res-binding-valid",
    "resource-return-collection-order-rt",
    "resource-return-identity-rt",
    "resource-return-ownership-valid",
    "resource-state-bare-param-valid",
    "resource-state-drop-valid",
    "resource-state-field-assign-valid",
    "resource-state-mutation-valid",
    "resource-state-return-rt",
    "resource-state-valid",
    "resource-union-drop-valid",
    "resource-union-foreach-valid",
    "resource-union-state-access-valid",
    "resource-union-state-drop-valid",
    "resource-union-valid",
    // rt-behavior/security
    "allocator-01-quick-bins",
    "allocator-02-size-overflow",
    "allocator-03-free-list-integrity",
    "allocator-05-grow-carve",
    "unicode-01-repeat-overflow",
    "unicode-02-pad-overflow",
    "unicode-03-ingress-utf8-invariant",
    "unicode-04-count-underread",
    "unicode-05-find-fold-parity",
    "unicode-06-find-negative-start",
    "unicode-07-padchar-scalar",
    "unicode-08-tobytes-roundtrip",
    "unicode-09-expanding-two-pass",
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
    "control-flow-inline-trap-valid",
    "error_block_adopt_paths",
    "func-bare-trap-loop-leak-rt",
    "func-bare-trap-rt",
    "inline-bare-trap-loop-leak-rt",
    "inline-bare-trap-propagate-rt",
    "inline-bare-trap-rt",
    "inline-trap-builtin-valid",
    "inline-trap-callback-member-rt",
    "inline-trap-default-able-types",
    "inline-trap-discard-error-rt",
    "inline-trap-fallible-member-rt",
    "inline-trap-fallthrough-recover-rt",
    "inline-trap-infallible-builtin-valid",
    "inline-trap-tostring-bytes-rt",
    "inline-trap-union-bind-rt",
    "trap-body-local-shadows-private-rt",
    "trap-function-inline-errors-rt",
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
    "vector-promotion",
    "vector-inline-ops",
    "vector-length-distance-inline-rt",
    "vector-native-carrier",
    "vector-normalize-inline-rt",
    // rt-error/arithmetic
    "arithmetic-add-sub-invalid-rt",
    "arithmetic-division-invalid-rt",
    "arithmetic-exp-mod-invalid-rt",
    "arithmetic-fixed-invalid-rt",
    "arithmetic-float-domain-rt",
    "arithmetic-float-fma-observed-rt",
    "arithmetic-float-mod-invalid-rt",
    "arithmetic-float-nan-rt",
    "arithmetic-float-overflow-rt",
    "arithmetic-multiplication-invalid-rt",
    "arithmetic-order-of-operations-invalid-rt",
    "bug137-fixed-pow-underflow-overflow-rt",
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
    "func_collection_findLastIndex_not_found",
    "func_collection_findLastIndex_out_of_range",
    "func_collection_find_not_found",
    "func_collection_find_out_of_range",
    "func_collection_get_not_found",
    "func_collection_get_out_of_range",
    "func_collection_insert_inplace_out_of_range",
    "func_collection_insert_out_of_range",
    "func_collection_mid_negative_range",
    "func_collection_mid_out_of_range",
    "func_collection_removeAt_inplace_out_of_range",
    "func_collection_removeAt_out_of_range",
    "func_collection_set_out_of_range",
    "func_collection_sum_overflow",
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
    "types-with-update-owned",
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
    // --- the rest of tests/rt-behavior ---
    //
    // The list above was generated with a cap of four fixtures per family,
    // which is a reasonable way to pick a BREADTH sample and a poor way to
    // reach a lowering only one program in a family has. These are the other
    // 220 rt-behavior fixtures that this harness can lower from
    // `src/main.mfb` alone -- everything except the package-bearing ones
    // (fixture_projects.rs owns those) and the ones with no `src/main.mfb`.
    //
    // They cost about a minute of suite time between them. What they buy is
    // whatever the cap dropped, which is not knowable from the outside: the cap
    // was applied per family, and coverage is not distributed per family.
    "mutate-split-rt",
    "tier-a-queries-rt",
    "tier-b-replace-rt",
    "tier-b-transforms-rt",
    "tomarkdown-flags-rt",
    "tomarkdown-fontsize-rt",
    "color_packed_rt",
    "color_perceptual_rt",
    "color_to_string_rt",
    "func_color_rgba_valid",
    "crypto-ed25519-malleability-invalid",
    "crypto-ed448-valid",
    "crypto-hpke-x25519-valid",
    "crypto-hpke-x448-valid",
    "crypto-kat-valid",
    "crypto-kdf-invalid",
    "crypto-randomint-wide-range-rt",
    "crypto-sha1-advisory-valid",
    "crypto-sha3-kat-valid",
    "crypto-x448-valid",
    "identifier-generators",
    "datetime-format-valid",
    "datetime-instant-valid",
    "datetime-invalid",
    "datetime-iso-nanos-rt",
    "datetime-parse-range-rt",
    "datetime-parse-trap-rt",
    "datetime-parse-valid",
    "datetime-withzone-instant-rt",
    "func_datetime_localOffset_valid",
    "fs-atomic-write",
    "fs-close-failed-rt",
    "fs-create-temp-file-rt",
    "fs-embedded-nul-rt",
    "fs-listdir-order-rt",
    "fs-nofollow-symlink-rt",
    "fs-path-errors-rt",
    "fs-pathjoin-rules-rt",
    "fs-readline-buffer-boundary-rt",
    "fs-temp-file-buffered",
    "fs-text-utf8-rt",
    "fs-write-bytes-payload-order-rt",
    "func_fs_appendBytes_valid",
    "func_fs_appendText_valid",
    "func_fs_canonicalPath_valid",
    "func_fs_close_valid",
    "func_fs_createDirectories_valid",
    "func_fs_createDirectory_valid",
    "func_fs_createTempFile_valid",
    "func_fs_currentDirectory_valid",
    "func_fs_deleteDirectory_valid",
    "func_fs_deleteFile_valid",
    "func_fs_directoryExists_valid",
    "func_fs_eof_valid",
    "func_fs_exists_valid",
    "func_fs_fileExists_valid",
    "func_fs_flush_valid",
    "func_fs_isBuffered_valid",
    "func_fs_isWithin_valid",
    "func_fs_listDirectory_valid",
    "func_fs_openFileNoFollow_valid",
    "func_fs_openFile_valid",
    "func_fs_openWithin_valid",
    "func_fs_open_valid",
    "func_fs_pathBaseName_valid",
    "func_fs_pathDirName_valid",
    "func_fs_pathExtension_valid",
    "func_fs_pathJoin_valid",
    "func_fs_pathNormalize_valid",
    "func_fs_readAllBytes_valid",
    "func_fs_readAll_valid",
    "func_fs_readBytes_valid",
    "func_fs_readLine_valid",
    "func_fs_readText_valid",
    "func_fs_setBuffered_valid",
    "func_fs_setCurrentDirectory_valid",
    "func_fs_writeAllBytes_valid",
    "func_fs_writeAll_valid",
    "func_fs_writeBytesAtomic_valid",
    "func_fs_writeBytes_valid",
    "func_fs_writeTextAtomic_valid",
    "func_fs_writeText_valid",
    "bug156_return_with_literal_coercion",
    "bug361_folded_literal_type_name",
    "builtin-predicate-as-value-rt",
    "codegen-conversion-edges-rt",
    "fixed-min-literal",
    "scalar-conversions-rt",
    "scalar-primitive-rt",
    "scalar-strings-seam-rt",
    "stdlib-error-code-contracts-rt",
    "func_http_route_valid",
    "http_server_loopback",
    "func_io_print_valid",
    "func_io_setBuffered_valid",
    "func_io_writeError_valid",
    "func_io_write_valid",
    "io-input-eof-buffering",
    "json-parse-deep-scalar-scan-rt",
    "qualified-union-variant-rt",
    "strings-artifact-coverage-rt",
    "strings-display-width-rt",
    "bug130_neon_exp_range_boundaries",
    "bug131_float_atan2_origin",
    "bug134_float_log_subnormal",
    "bug137_pow_negative_zero",
    "bug137_rand_unbiased_bounds",
    "bug164_exp_large_argument_saturation",
    "bug74_pow_operator_base_clobber",
    "ceil-fixed-vector-overflow-rt",
    "math_package_valid",
    "math_simd_signzero_tail_valid",
    "record-field-args",
    "round-ties-away-boundary-rt",
    "money_tostring_mode_decoupled",
    "func_net_toUrl_invalid_runtime",
    "func_net_toUrl_valid",
    "func_net_url_toString_valid",
    "qualified-enum-member-rt",
    "func_os_executablePath_valid",
    "func_os_getEnvOr_valid",
    "func_os_getEnv_valid",
    "func_os_hasEnv_valid",
    "func_os_hostName_valid",
    "func_os_name_valid",
    "func_os_pid_valid",
    "func_os_resourcePath_valid",
    "func_os_setEnv_valid",
    "func_os_system_status_valid",
    "func_os_unsetEnv_valid",
    "func_os_userName_valid",
    "os-args-basic",
    "os-env-roundtrip",
    "os-environ-roundtrip",
    "os-identity-queries",
    "os-introspect-basic",
    "os-sleep-main-rt",
    "poll",
    "receive-lines",
    "receivebytes",
    "send-grep",
    "send-timeout",
    "sendbytes",
    "shell-exitcode",
    "signal",
    "spawn-fail-trap",
    "spawn-waitfor",
    "spawnenv",
    "project-entry-func-default-trap",
    "project-entry-func-main-trap",
    "project-entry-func-named-args-valid",
    "project-entry-func-trap",
    "project-entry-param-trap",
    "project-entry-sub-args-default-trap",
    "project-entry-sub-args-main-trap",
    "project-entry-sub-default-trap",
    "project-entry-sub-main-trap",
    "regex-find-absence-rt",
    "replace-empty-pattern-rt",
    "strings-empty-needle-rt",
    "func_tcp_connect_valid",
    "func_tcp_localAddress_valid",
    "func_tcp_poll_valid",
    "func_tcp_readText_valid",
    "func_tcp_read_valid",
    "func_tcp_remoteAddress_valid",
    "func_tcp_setReadTimeout_valid",
    "func_tcp_setWriteTimeout_valid",
    "func_tcp_stream_valid",
    "func_tcp_writeText_valid",
    "func_tcp_write_valid",
    "tcp-accept-timeout-convention-rt",
    "tcp-bounded-accept-blocking-rt",
    "tcp-connect-timeout-convention-rt",
    "tcp-poll-list-rt",
    "tcp-poll-timeout-convention-rt",
    "tcp-read-eof-raises-rt",
    "tcp-readtimeout-convention-rt",
    "tcp-udp-poll-list-trap-rt",
    "tcp-write-peer-closed-raises-rt",
    "func_term_drawHLine_valid",
    "func_term_drawText_attr_valid",
    "func_term_drawText_valid",
    "func_term_drawVLine_valid",
    "func_term_draw_wide_valid",
    "func_term_fillRect_valid",
    "func_term_grid_diff_valid",
    "func_term_grid_draw_valid",
    "func_term_inactive_gate_valid",
    "func_term_sync_valid",
    "func_term_wide_glyph_valid",
    "term-styling-basic",
    "tls-read-timeout-rt",
    "tls-timeout-convention-rt",
    "func_udp_receive_valid",
    "func_udp_send_valid",
];

/// The program's own functions — everything the source declared, plus the
/// synthesized bodies of the builtin packages it imported.
///
/// Runtime helpers (`runtime.*`), constructors and the per-backend program entry
/// are excluded: those legitimately differ, and including them would turn a real
/// disagreement into noise nobody reads.
fn program_functions(plan: &NativeCodePlan) -> BTreeSet<String> {
    plan.functions
        .iter()
        .map(|f| f.name.clone())
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
        let mut agreed: Option<(CodeTarget, BTreeSet<String>)> = None;
        for target in CodeTarget::ALL {
            let plan = try_code_for_src(&source, target, crate::target::NativeBuildMode::Console)
                .unwrap_or_else(|err| panic!("{fixture} on {}: {err}", target.name()));
            assert_eq!(
                plan.target,
                target.name(),
                "{fixture}: the code plan must record the backend it was lowered for"
            );
            let functions = program_functions(&plan);
            assert!(
                !functions.is_empty(),
                "{fixture}: {} emitted no program functions at all",
                target.name()
            );
            match &agreed {
                None => agreed = Some((target, functions)),
                Some((first, expected)) => {
                    let missing: Vec<&String> = expected.difference(&functions).collect();
                    let extra: Vec<&String> = functions.difference(expected).collect();
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
        CORPUS.len() >= 620,
        "the corpus is {} fixtures; it was generated at 421 across 88 families, then grew to 620 with the rest of tests/rt-behavior, \
         and a list that shrinks silently is a gate that stops measuring",
        CORPUS.len()
    );
    // Every fixture, then report. Stopping at the first failure means one bad
    // row hides the rest, and this list is long enough that finding them one
    // run at a time is the difference between an afternoon and a minute.
    let mut failed = Vec::new();
    for fixture in CORPUS {
        let source = fixture_src(fixture);
        let plan = match try_code_for_src(
            &source,
            CodeTarget::LinuxX86_64,
            crate::target::NativeBuildMode::Console,
        ) {
            Ok(plan) => plan,
            Err(err) => {
                failed.push(format!("{fixture}: {}", err.lines().next().unwrap_or(&err)));
                continue;
            }
        };
        for f in &plan.functions {
            if f.instructions.len() <= 1 {
                failed.push(format!(
                    "{fixture}: `{}` lowered to {} instruction(s) — a body that \
                     is only its entry label is a lowering that declined",
                    f.name,
                    f.instructions.len()
                ));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{} corpus fixture(s) did not lower:\n  {}",
        failed.len(),
        failed.join("\n  ")
    );
}
