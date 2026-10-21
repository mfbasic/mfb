//! The rest of the single-file fixtures this harness can lower.
//!
//! `corpus.rs` lowers 620 committed fixtures. These are the single-file programs
//! under `tests/rt-behavior/**` that were left out of that list and that also
//! lower from `src/main.mfb` alone: language and library shapes (closures over
//! resources, inline `TRAP` positions, thread transfer of unions and maps,
//! generic instantiations, overload resolution, scope and move rules) whose
//! codegen paths no program in the other list takes.
//!
//! They are held to the same two contracts as the corpus. `try_code_for_src`
//! validates as it lowers - a branch to an undefined label, a relocation against a
//! symbol nothing defines, or a data object nobody emitted is rejected - and no
//! function may lower to just its entry label, which is what a lowering that
//! silently declined looks like (the validator accepts it, and a behavioural test
//! that does not happen to call it never notices).
//!
//! The list is explicit and its length is asserted, so a fixture that stops
//! lowering fails here instead of quietly dropping out. The 15 fixtures of this
//! family that are NOT in it each need something this harness has no way to
//! supply from `src/main.mfb` alone: a second source file or an imported `.mfp` /
//! source package (14), or a native library (`native-declared-resource-union-rt`).

use crate::testutil::{fixture_src, try_code_for_src, CodeTarget};

const REST: &[&str] = &[
    "bug754-float-overflow-raises-inside-expression-rt",
    "bug762-fixed-times-overloaded-builtin-rt",
    "bug762-float-arith-on-closure-call-rt",
    "bug762-float-arith-on-general-call-rt",
    "bug762-float-arith-on-overloaded-builtin-rt",
    "bug762-float-arith-on-runtime-call-rt",
    "bug762-float-arith-on-user-call-rt",
    "bug762-float-negate-general-call-rt",
    "bug762-money-times-collection-sum-rt",
    "float-call-boundary-finite-rt",
    "float-fixed-comparison-promotion",
    "money-min-integer-divisor-rounding",
    "bug752-res-element-callbacks-rt",
    "builtin-callback-isolated-widening-rt",
    "callback-string-return-identity-rt",
    "empty-list-literal-argument-rt",
    "func_collection_repeat",
    "inline-list-element-type-rt",
    "list-set-shared-offset-rt",
    "negative-index-rule-rt",
    "reduce-result-is-seed-rt",
    "union-variant-element-insert-rt",
    "union-variant-reduce-rt",
    "compress-crc32-valid",
    "compress-decode-valid",
    "compress-encode-roundtrip-valid",
    "bug764_enum_case_type_name_collision",
    "exit-for-named-rt",
    "for-mixed-numeric-operands",
    "loop-do-until-mut-integer",
    "loop-do-until-mut-string",
    "loop-do-until-tostring-condition",
    "loop-do-until-uninitialized-string",
    "loop-do-while-mut-string",
    "loop-for-mut-string",
    "loop-for-uninitialized-string",
    "loop-while-concat-condition",
    "loop-while-integer-condition",
    "loop-while-mut-string",
    "loop-while-tostring-condition",
    "loop-while-uninitialized-string",
    "match-fixed-money-literal-patterns",
    "match-float-signed-zero",
    "stack-overflow-rt",
    "fixed-tostring-rounds-near-max",
    "fixed_literal_maximum",
    "float-suffixed-literal-into-fixed",
    "crypto-argon2id-valid",
    "crypto-open-sealed-aad-rt",
    "func_csv_parse_file_valid",
    "func_csv_readRow_valid",
    "datetime-arith-errors-rt",
    "datetime-offset-seconds-rt",
    "datetime-parse-strict-rt",
    "datetime-tz-new-york-rt",
    "encoding-codec-self-update-rt",
    "encoding-decode-errors-rt",
    "fs-create-directories-native-separators-rt",
    "fs-pathjoin-absolute-owned-root-rt",
    "fs-readline-final-bare-cr-rt",
    "fs-self-update-borrow-rt",
    "func_fs_openData_valid",
    "func_fs_seek_valid",
    "pathdirname_constant_owned",
    "entry-func-nothing-fail-rt",
    "entry-func-nothing-valid",
    "func_overload_value_valid",
    "func_override_toint_enum_precedence",
    "func_override_tostring_enum_precedence",
    "isolated-callable-call-rt",
    "isolated-callable-widening-rt",
    "string_global_default_owned",
    "toInt_enum",
    "toString_enum",
    "tostring_boolean_self_append",
    "tostring_string_owning_store",
    "typename_user_call",
    "io-input-prompt-borrow-rt",
    "json-parse-reviver-isolated-rt",
    "literal-spans-lines-valid",
    "bug659_fixed_exp_large_argument_reduction",
    "bug659_fixed_pow_fractional_product_wrap",
    "fixed-pow-negative-whole-exponent-small-result-rt",
    "closure-capture-resource-append-rt",
    "owned-argument-semantics-rt",
    "money_fixed_rounding",
    "bug979_toUrl_host_canonical",
    "func_os_appCachePath_valid",
    "func_os_appDataPath_valid",
    "func_os_appResourcePath_reads_resource",
    "func_os_getConfigOr_valid",
    "func_os_prog_valid",
    "func_os_userDocumentsPath_valid",
    "func_os_userHomePath_valid",
    "os-self-update-borrow-rt",
    "regex-surface-parity-rt",
    "append-res-list-literal-rt",
    "bug1013_state_record_rt",
    "bug1027_member_into_module_union_rt",
    "bug1030_state_lambda_param_rt",
    "bug1030_state_top_level_only_rt",
    "bug1030_state_top_level_rt",
    "bug1054_func_ref_state_rt",
    "bug487_state_mutating_operand",
    "bug643-collection-helpers-keep-caller-handles-rt",
    "float-through-repackaging-collections-rt",
    "loop-body-close-resource-rt",
    "param-resource-inferred-rt",
    "resource-owner-scope-rt",
    "resource-temporary-closed-rt",
    "resource-union-box-owner-rt",
    "state-assign-from-local-copies-valid",
    "state-field-source-generic-arg-valid",
    "float-arith-on-global-and-state-field-valid",
    "global-init-calls-builtin-tables-valid",
    "global-mut-assign-in-function-valid",
    "grapheme-indic-context-rt",
    "line-breaks-valid",
    "nfc-blocked-composition-rt",
    "self-update-grow-valid",
    "self-update-rewrite-valid",
    "self-update-window-valid",
    "strings-inline-trap-fallible-rt",
    "strings-pad-to-width-rt",
    "thread-start-inline-trap-rt",
    "bug630-nested-thread-handle-rt",
    "bug691-res-thread-param",
    "bug692-record-thread-copy",
    "bug693-worker-state",
    "thread-borrow-on-pass",
    "thread-enum-message-rt",
    "thread-pool-list",
    "thread-transfer-inline-trap-fail-drop-rt",
    "thread-transfer-parent-view",
    "thread-view-state-independent",
    "thread-waitfor-repeatable",
    "thread-worker-state-send-rt",
    "threadworker-res-binding-rt",
    "tls-write-after-read-closed-raises-rt",
    "tls-write-peer-closed-raises-rt",
    "bug644-inline-trap-float-sum-overflow-rt",
    "inline-trap-call-free-operator-chain-rt",
    "inline-trap-fixed-comparison-promotion-rt",
    "inline-trap-fixed-comparison-short-circuit-rt",
    "inline-trap-fixed-comparison-short-circuit-selected-rt",
    "inline-trap-handler-routing-rt",
    "inline-trap-unproved-builtin-rt",
    "trap-local-initialized-before-failure-rt",
    "enum-default-first-variant-rt",
    "union-container-conversion-rt",
    "union-variant-expression-wrap-resource",
    "union-variant-expression-wrap",
    "with-fallible-comparison-source-order",
    "vector-native-inline-call-arg-rt",
];

#[test]
fn every_remaining_fixture_lowers_and_no_function_is_empty() {
    assert!(
        REST.len() >= 154,
        "the list is {} fixtures; it was written at 154, and a list that shrinks \
         silently is a gate that stops measuring",
        REST.len()
    );
    let mut failed = Vec::new();
    for fixture in REST {
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
                    "{fixture}: `{}` lowered to {} instruction(s) - a body that is only \
                     its entry label is a lowering that declined",
                    f.name,
                    f.instructions.len()
                ));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{} fixture(s) did not lower:\n  {}",
        failed.len(),
        failed.join("\n  ")
    );
}
