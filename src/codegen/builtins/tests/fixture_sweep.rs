//! Every committed `rt-behavior` fixture that `corpus.rs` and `fixture_projects.rs`
//! do not already lower, lowered in process for Linux x86-64.
//!
//! The acceptance matrix builds each fixture with a spawned `mfb` and so cannot be
//! observed from here; a program that the front end accepts but a builder refuses
//! ("native code ... is not supported") is a build that dies for the author with no
//! way to work around it. These programs are the regression fixtures of the bugs
//! the engine's rarest routes were written for (STATE payloads, closures capturing
//! strings and resources, callback hand-over, thread transfer, codec self-updates,
//! datetime and crypto arguments), so lowering them is what puts those routes under
//! the in-process unit suite.
//!
//! The list is explicit, and its length is asserted, for the reason `corpus.rs`
//! gives: discovering fixtures and skipping the ones that fail to lower would let
//! the sweep hollow out silently. Three fixtures are not here: `enum_pkg_tiny`
//! names a directory that exists twice under `tests/` (`fixture_dir` refuses to
//! guess), and `private-overload-beside-export-rt` and
//! `template-in-later-file-import-alias-rt` need the project's own dependency
//! build, which the in-process loader does not run.
//!
//! The sweep is split into shards so the test runner lowers them in parallel.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{try_code_for_fixture_project, CodeTarget};

const FIXTURES: &[&str] = &[
    "allocator-04-thread-arena-init",
    "append-res-list-literal-rt",
    "bug1013_state_record_rt",
    "bug1027_member_into_module_union_rt",
    "bug1030_state_lambda_param_rt",
    "bug1030_state_top_level_only_rt",
    "bug1030_state_top_level_rt",
    "bug104_aliased_overload_import",
    "bug1054_func_ref_state_rt",
    "bug221_transfer_accept_named_args",
    "bug487_state_mutating_operand",
    "bug630-nested-thread-handle-rt",
    "bug643-collection-helpers-keep-caller-handles-rt",
    "bug644-inline-trap-float-sum-overflow-rt",
    "bug659_fixed_exp_large_argument_reduction",
    "bug659_fixed_pow_fractional_product_wrap",
    "bug691-res-thread-param",
    "bug692-record-thread-copy",
    "bug693-worker-state",
    "bug752-res-element-callbacks-rt",
    "bug754-float-overflow-raises-inside-expression-rt",
    "bug762-fixed-times-overloaded-builtin-rt",
    "bug762-float-arith-on-closure-call-rt",
    "bug762-float-arith-on-general-call-rt",
    "bug762-float-arith-on-overloaded-builtin-rt",
    "bug762-float-arith-on-runtime-call-rt",
    "bug762-float-arith-on-user-call-rt",
    "bug762-float-negate-general-call-rt",
    "bug762-money-times-collection-sum-rt",
    "bug764_enum_case_type_name_collision",
    "bug979_toUrl_host_canonical",
    "builtin-callback-isolated-widening-rt",
    "builtin_binding_pkg",
    "builtin_type_pkg",
    "callback-string-return-identity-rt",
    "closure-capture-resource-append-rt",
    "compress-crc32-valid",
    "compress-decode-valid",
    "compress-encode-roundtrip-valid",
    "config_pkg",
    "crypto-argon2id-valid",
    "crypto-open-sealed-aad-rt",
    "datetime-arith-errors-rt",
    "datetime-offset-seconds-rt",
    "datetime-parse-strict-rt",
    "datetime-tz-new-york-rt",
    "empty-list-literal-argument-rt",
    "encoding-codec-self-update-rt",
    "encoding-decode-errors-rt",
    "entry-func-nothing-fail-rt",
    "entry-func-nothing-valid",
    "enum-default-first-variant-rt",
    "exit-for-named-rt",
    "fixed-pow-negative-whole-exponent-small-result-rt",
    "fixed-tostring-rounds-near-max",
    "fixed_literal_maximum",
    "float-arith-on-global-and-state-field-valid",
    "float-call-boundary-finite-rt",
    "float-fixed-comparison-promotion",
    "float-suffixed-literal-into-fixed",
    "float-through-repackaging-collections-rt",
    "for-mixed-numeric-operands",
    "fs-create-directories-native-separators-rt",
    "fs-pathjoin-absolute-owned-root-rt",
    "fs-readline-final-bare-cr-rt",
    "fs-self-update-borrow-rt",
    "func_collection_repeat",
    "func_csv_parse_file_valid",
    "func_csv_readRow_valid",
    "func_fs_openData_valid",
    "func_fs_seek_valid",
    "func_os_appCachePath_valid",
    "func_os_appDataPath_valid",
    "func_os_appResourcePath_reads_resource",
    "func_os_getConfigOr_valid",
    "func_os_prog_valid",
    "func_os_userDocumentsPath_valid",
    "func_os_userHomePath_valid",
    "func_overload_value_valid",
    "func_override_toint_enum_precedence",
    "func_override_tostring_enum_precedence",
    "func_override_visibility",
    "func_thread_closeStdIn_valid",
    "func_thread_result_valid",
    "func_thread_transfer_valid",
    "global-init-calls-builtin-tables-valid",
    "global-mut-assign-in-function-valid",
    "grapheme-indic-context-rt",
    "holder_pkg",
    "inline-list-element-type-rt",
    "inline-trap-call-free-operator-chain-rt",
    "inline-trap-fixed-comparison-promotion-rt",
    "inline-trap-fixed-comparison-short-circuit-rt",
    "inline-trap-fixed-comparison-short-circuit-selected-rt",
    "inline-trap-handler-routing-rt",
    "inline-trap-unproved-builtin-rt",
    "io-input-prompt-borrow-rt",
    "isolated-callable-call-rt",
    "isolated-callable-widening-rt",
    "json-parse-reviver-isolated-rt",
    "libsnd-load-sound-rt",
    "libsnd-open-file-info-rt",
    "libsnd-playback-rt",
    "libsnd-read-samples-rt",
    "line-breaks-valid",
    "list-set-shared-offset-rt",
    "literal-spans-lines-valid",
    "local-names-distinct-from-top-level-valid",
    "loop-body-close-resource-rt",
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
    "mfp-package-dependency-rt",
    "money-min-integer-divisor-rounding",
    "money_fixed_rounding",
    "native-cbuffer-read-rt",
    "native-closed-guard-rt",
    "native-declared-resource-union-rt",
    "native-link-alias-collision-rt",
    "native-link-and-truthy-rt",
    "native-link-const-64bit-rt",
    "native-link-free-rt",
    "native-link-import-sqlite-rt",
    "native-link-inline-trap-rt",
    "native-link-nested-success-rt",
    "native-link-sqlite-rt",
    "native-private-resource-rt",
    "native-resource-in-collection-rt",
    "native-resource-scope-drop-rt",
    "native-resource-state-import-rt",
    "native-resource-state-rt",
    "native-resource-thread-accept-rt",
    "native-resource-transfer-fail-usable-rt",
    "native-stateless-record-rt",
    "native-struct-cstring-rt",
    "native-struct-scalar-rt",
    "negative-index-rule-rt",
    "nfc-blocked-composition-rt",
    "os-env-thread-race-rt",
    "os-self-update-borrow-rt",
    "os-sleep-worker-cancel-rt",
    "os-sleep-worker-rt",
    "owned-argument-semantics-rt",
    "p121d-state-reach-rt",
    "package-builtin-binding-init-rt",
    "package-builtin-type-export-rt",
    "param-resource-inferred-rt",
    "pathdirname_constant_owned",
    "privovl_pkg",
    "project-entry-func-args-foobar-trap",
    "project-entry-func-foobar-trap",
    "project-entry-nothing-args-foobar-trap",
    "project-entry-nothing-foobar-trap",
    "project-fs-createTempFile-package-valid",
    "project-record-comparable-package-valid",
    "project-with-package-import-as",
    "record-res-field-export-rt",
    "reduce-result-is-seed-rt",
    "regex-surface-parity-rt",
    "resource-owner-scope-rt",
    "resource-state-import-rt",
    "resource-temporary-closed-rt",
    "resource-union-box-owner-rt",
    "scopepkg",
    "self-update-grow-valid",
    "self-update-rewrite-valid",
    "self-update-window-valid",
    "source-package-dependency-rt",
    "source-package-get-config-or-rt",
    "source-package-isolated-rt",
    "source_pkg_tiny",
    "source_pkg_worker",
    "stack-overflow-rt",
    "state-assign-from-local-copies-valid",
    "state-field-source-generic-arg-valid",
    "state_reach_worker",
    "string_global_default_owned",
    "strings-inline-trap-fallible-rt",
    "strings-pad-to-width-rt",
    "template-in-later-file-error-source-rt",
    "template-in-later-file-private-scope-rt",
    "thread-borrow-on-pass",
    "thread-bounded-queues",
    "thread-drop-cleanup",
    "thread-dual-cancel",
    "thread-enum-message-rt",
    "thread-error-source-rt",
    "thread-fixed-list-transfer-rt",
    "thread-fs-close-rt",
    "thread-fs-listdir-order-rt",
    "thread-fs-pathjoin-rt",
    "thread-fs-read-return",
    "thread-fs-readtext-return",
    "thread-import-package-print",
    "thread-import-pkg-receive-rt",
    "thread-link-worker-rt",
    "thread-main-poll",
    "thread-package-fanout-rt",
    "thread-package-globals-rt",
    "thread-pool-list",
    "thread-print-count",
    "thread-queue-timeout-cancel",
    "thread-receive-print",
    "thread-regex-rt",
    "thread-resource-transfer-fail-leak",
    "thread-resource-transfer-fail-reclaim",
    "thread-return-byte",
    "thread-return-fixed",
    "thread-return-float",
    "thread-return-integer",
    "thread-return-list-of-string",
    "thread-return-map-of-string-to-string",
    "thread-return-string",
    "thread-return-type",
    "thread-return-union",
    "thread-send-file-ownership-rt",
    "thread-start-inline-trap-rt",
    "thread-start-invalid-limit-trapped",
    "thread-strings-split-return",
    "thread-timeout-convention-rt",
    "thread-transfer-bidirectional-rt",
    "thread-transfer-inline-trap-fail-drop-rt",
    "thread-transfer-parent-view",
    "thread-transfer-state-rt",
    "thread-transfer-tcp-listener-rt",
    "thread-transfer-tls-socket-rt",
    "thread-transfer-union-state-rt",
    "thread-transfer-union-stateless-rt",
    "thread-view-state-independent",
    "thread-waitfor-repeatable",
    "thread-worker-state-send-rt",
    "threadworker-res-binding-rt",
    "tls-write-after-read-closed-raises-rt",
    "tls-write-peer-closed-raises-rt",
    "tmpl_scope_a",
    "tmpl_scope_f",
    "toInt_enum",
    "toInt_enum_package",
    "toString_enum",
    "toString_enum_package",
    "tostring_boolean_self_append",
    "tostring_string_owning_store",
    "trap-builtin-consumer",
    "trap-local-initialized-before-failure-rt",
    "type-template-in-later-file-private-rt",
    "typename_user_call",
    "union-container-conversion-rt",
    "union-variant-element-insert-rt",
    "union-variant-expression-wrap",
    "union-variant-expression-wrap-resource",
    "union-variant-reduce-rt",
    "vector-native-inline-call-arg-rt",
    "with-fallible-comparison-source-order",
];

const SHARDS: usize = 8;

fn lower_shard(shard: usize) {
    // The code stage recurses over the value tree; a libtest thread's 2 MiB is not
    // what the compiler runs on.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let mut failures = Vec::new();
            for name in FIXTURES.iter().skip(shard).step_by(SHARDS) {
                if let Err(error) =
                    try_code_for_fixture_project(name, CodeTarget::LinuxX86_64, Console)
                {
                    failures.push(format!("{name}: {}", error.lines().next().unwrap_or("")));
                }
            }
            assert!(
                failures.is_empty(),
                "fixtures the front end accepts but the backend refuses:\n  {}",
                failures.join("\n  ")
            );
        })
        .expect("spawn the lowering thread")
        .join()
        .unwrap_or_else(|_| panic!("a fixture took the lowering thread down"));
}

#[test]
fn the_fixture_list_is_the_size_it_was_written_for() {
    assert_eq!(FIXTURES.len(), 264);
    let mut sorted = FIXTURES.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), FIXTURES.len(), "a fixture is listed twice");
}

#[test]
fn fixtures_lower_shard_0() {
    lower_shard(0);
}

#[test]
fn fixtures_lower_shard_1() {
    lower_shard(1);
}

#[test]
fn fixtures_lower_shard_2() {
    lower_shard(2);
}

#[test]
fn fixtures_lower_shard_3() {
    lower_shard(3);
}

#[test]
fn fixtures_lower_shard_4() {
    lower_shard(4);
}

#[test]
fn fixtures_lower_shard_5() {
    lower_shard(5);
}

#[test]
fn fixtures_lower_shard_6() {
    lower_shard(6);
}

#[test]
fn fixtures_lower_shard_7() {
    lower_shard(7);
}
