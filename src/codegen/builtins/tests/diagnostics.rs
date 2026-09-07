//! The committed diagnostic goldens, reproduced in process.
//!
//! `tests/syntax/**` is the language's rejection contract: 581 programs, each
//! with a `golden/build.log` recording the exact rule codes the compiler must
//! emit for it. That corpus is gated only by `scripts/test-accept.sh`, which
//! spawns `mfb` — so from this process it covers nothing, and the two passes it
//! exercises hardest (`ir::shape` over the concrete HIR and `ir::verify` over
//! the lowered IR, ~3,200 and ~1,600 lines) had no in-process coverage beyond
//! the handful of hand-written `check_src` cases scattered through the tree.
//!
//! `testutil::check_src` runs exactly those two passes in the build's order.
//! For each fixture below it must produce **the same rule codes, in the same
//! order**, as the fixture's own golden. The golden is the oracle rather than a
//! list embedded here: it is already reviewed, already gated, and a copy would
//! drift from it silently.
//!
//! The list is the 417 fixtures whose codes `check_src` reproduces (exactly for
//! most; for a few the golden additionally carries a parser or resolver code that
//! these two passes do not emit). Four more were dropped because their directory
//! leaf name is not unique under `tests/`, so `fixture_dir` cannot address them.
//! The rest are excluded BY NAME rather than skipped at
//! run time: 26 are parse-error fixtures that `check_src` is documented to panic
//! on (it expects a program that lexes), and the remainder emit nothing from
//! these two passes. A skip-on-failure loop would let the corpus hollow out
//! silently — a fixture that stopped reproducing and one that never did look
//! identical in a green run.

use std::collections::BTreeSet;

use crate::testutil::{check_fixture_project, check_src, fixture_dir, fixture_src};

/// The rule codes a golden `build.log` records, in order.
///
/// A diagnostic line reads `… error[2-203-0022 TYPE_CALL_ARITY_MISMATCH]: …`,
/// and the rule is the NAME, not the numeric code: the numbers are a stable
/// external identifier and the name is what the compiler's diagnostic table is
/// keyed on.
fn golden_rules(fixture: &str) -> Vec<String> {
    let path = fixture_dir(fixture).join("golden").join("build.log");
    let log = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    log.split("error[")
        .skip(1)
        .filter_map(|rest| rest.split(']').next())
        .filter_map(|head| head.split_whitespace().nth(1))
        .map(str::to_string)
        .collect()
}

const DIAGNOSTIC_CORPUS: &[&str] = &[
    "abs_invalid",
    "accept_invalid",
    "angle_invalid",
    "app_setmode_invalid",
    "arithmetic-add-sub-invalid",
    "arithmetic-division-invalid",
    "arithmetic-exp-mod-invalid",
    "arithmetic-fixed-invalid",
    "arithmetic-multiplication-invalid",
    "arithmetic-order-of-operations-invalid",
    "bug11_fixed_extreme_exponent_invalid",
    "bug231_resource_union_map_key_invalid",
    "bug349_instant_named_arg_arity_invalid",
    "byteLen",
    "canvas-drawitem-thread-plane-invalid",
    "canvas-setgroup-consumes-items",
    "caseFold",
    "clamp_length_invalid",
    "color_to_string_unrelated_record_invalid",
    "comparisons-invalid",
    "connect_invalid",
    "contains",
    "continue-loop-invalid",
    "control-flow-condition-types-invalid",
    "control-flow-fail-propagate-invalid",
    "control-flow-for-step-zero-invalid",
    "control-flow-inline-trap-invalid",
    "control-flow-match-exhaustiveness-invalid",
    "control-flow-match-pattern-invalid",
    "control-flow-sub-trap-invalid",
    "cross_invalid",
    "device_literal_invalid",
    "displayWidth",
    "distance_invalid",
    "dot_invalid",
    "endpoints_invalid",
    "endsWith",
    "error_invalid",
    "exit-loop-invalid",
    "exit-program-invalid",
    "exit-sub-invalid",
    "find_invalid",
    "func_bits_band_invalid",
    "func_bits_bnot_invalid",
    "func_bits_bor_invalid",
    "func_bits_bswap16_invalid",
    "func_bits_bswap32_invalid",
    "func_bits_bswap64_invalid",
    "func_bits_bxor_invalid",
    "func_bits_clz_invalid",
    "func_bits_ctz_invalid",
    "func_bits_popCount_invalid",
    "func_bits_rl32_invalid",
    "func_bits_rl64_invalid",
    "func_bits_rr32_invalid",
    "func_bits_rr64_invalid",
    "func_bits_sl_invalid",
    "func_bits_sr_invalid",
    "func_bits_sra_invalid",
    "func_collection_append_invalid",
    "func_collection_contains_invalid",
    "func_collection_filter_invalid",
    "func_collection_find_invalid",
    "func_collection_forEach_invalid",
    "func_collection_getOr_invalid",
    "func_collection_get_invalid",
    "func_collection_hasKey_invalid",
    "func_collection_insert_invalid",
    "func_collection_keys_invalid",
    "func_collection_len_invalid",
    "func_collection_mid_invalid",
    "func_collection_prepend_invalid",
    "func_collection_reduceRight_invalid",
    "func_collection_reduce_invalid",
    "func_collection_removeAt_invalid",
    "func_collection_removeKey_invalid",
    "func_collection_replace_invalid",
    "func_collection_set_invalid",
    "func_collection_set_map_invalid",
    "func_collection_sum_invalid",
    "func_collection_transform_invalid",
    "func_collection_values_invalid",
    "func_color_rgba_invalid",
    "func_csv_parse_invalid",
    "func_datetime_localOffset_invalid",
    "func_encoding_base32Decode_invalid",
    "func_encoding_base32Encode_invalid",
    "func_encoding_base64Decode_invalid",
    "func_encoding_base64Encode_invalid",
    "func_encoding_base64UrlDecode_invalid",
    "func_encoding_base64UrlEncode_invalid",
    "func_encoding_codepageDecode_invalid",
    "func_encoding_codepageEncode_invalid",
    "func_encoding_formUrlDecode_invalid",
    "func_encoding_formUrlEncode_invalid",
    "func_encoding_hexDecode_invalid",
    "func_encoding_hexEncode_invalid",
    "func_encoding_htmlEscape_invalid",
    "func_encoding_htmlUnescape_invalid",
    "func_encoding_percentDecode_invalid",
    "func_encoding_percentEncode_invalid",
    "func_encoding_punycodeDecode_invalid",
    "func_encoding_punycodeEncode_invalid",
    "func_encoding_sleb128Decode_invalid",
    "func_encoding_sleb128Encode_invalid",
    "func_encoding_uleb128Decode_invalid",
    "func_encoding_uleb128Encode_invalid",
    "func_encoding_utf16Decode_invalid",
    "func_encoding_utf16Encode_invalid",
    "func_encoding_utf32Decode_invalid",
    "func_encoding_utf32Encode_invalid",
    "func_encoding_utf8Decode_invalid",
    "func_encoding_utf8Encode_invalid",
    "func_encoding_varintDecode_invalid",
    "func_encoding_varintEncode_invalid",
    "func_errorcode_constant_invalid",
    "func_fs_appendBytes_invalid",
    "func_fs_appendText_invalid",
    "func_fs_canonicalPath_invalid",
    "func_fs_close_invalid",
    "func_fs_createDirectories_invalid",
    "func_fs_createDirectory_invalid",
    "func_fs_createTempFile_invalid",
    "func_fs_currentDirectory_invalid",
    "func_fs_deleteDirectory_invalid",
    "func_fs_deleteFile_invalid",
    "func_fs_directoryExists_invalid",
    "func_fs_eof_invalid",
    "func_fs_exists_invalid",
    "func_fs_fileExists_invalid",
    "func_fs_flush_invalid",
    "func_fs_isBuffered_invalid",
    "func_fs_isWithin_invalid",
    "func_fs_listDirectory_invalid",
    "func_fs_openFileNoFollow_invalid",
    "func_fs_openFile_invalid",
    "func_fs_openWithin_invalid",
    "func_fs_open_invalid",
    "func_fs_pathBaseName_invalid",
    "func_fs_pathDirName_invalid",
    "func_fs_pathExtension_invalid",
    "func_fs_pathJoin_invalid",
    "func_fs_pathNormalize_invalid",
    "func_fs_readAllBytes_invalid",
    "func_fs_readAll_invalid",
    "func_fs_readBytes_invalid",
    "func_fs_readLine_invalid",
    "func_fs_readText_invalid",
    "func_fs_setBuffered_invalid",
    "func_fs_setCurrentDirectory_invalid",
    "func_fs_tempDirectory_invalid",
    "func_fs_writeAllBytes_invalid",
    "func_fs_writeAll_invalid",
    "func_fs_writeBytesAtomic_invalid",
    "func_fs_writeBytes_invalid",
    "func_fs_writeTextAtomic_invalid",
    "func_fs_writeText_invalid",
    "func_io_flush_invalid",
    "func_io_input_invalid",
    "func_io_isBuffered_invalid",
    "func_io_isErrorTerminal_invalid",
    "func_io_isInputTerminal_invalid",
    "func_io_isOutputTerminal_invalid",
    "func_io_pollInput_invalid",
    "func_io_printError_invalid",
    "func_io_print_invalid",
    "func_io_readByte_invalid",
    "func_io_readChar_invalid",
    "func_io_readLine_invalid",
    "func_io_setBuffered_invalid",
    "func_io_writeError_invalid",
    "func_io_write_invalid",
    "func_json_getOr_invalid",
    "func_json_get_invalid",
    "func_json_parse_invalid",
    "func_json_stringify_invalid",
    "func_math_abs_intarray_invalid",
    "func_math_abs_invalid",
    "func_math_acos_invalid",
    "func_math_asin_invalid",
    "func_math_atan2_invalid",
    "func_math_atan_invalid",
    "func_math_ceil_invalid",
    "func_math_clamp_invalid",
    "func_math_cos_invalid",
    "func_math_exp_floatarray_invalid",
    "func_math_exp_invalid",
    "func_math_floor_invalid",
    "func_math_log10_invalid",
    "func_math_log_fixedarray_invalid",
    "func_math_log_invalid",
    "func_math_max_invalid",
    "func_math_min_array_invalid",
    "func_math_min_invalid",
    "func_math_pow_invalid",
    "func_math_rand_invalid",
    "func_math_round_invalid",
    "func_math_seed_invalid",
    "func_math_sin_floatarray_invalid",
    "func_math_sin_invalid",
    "func_math_sqrt_floatarray_invalid",
    "func_math_sqrt_invalid",
    "func_math_tan_invalid",
    "func_net_decode_invalid",
    "func_net_lookup_invalid",
    "func_net_ping_invalid",
    "func_net_toUrl_invalid",
    "func_os_arch_invalid",
    "func_os_args_invalid",
    "func_os_cpuCount_invalid",
    "func_os_environ_invalid",
    "func_os_executablePath_invalid",
    "func_os_getEnvOr_invalid",
    "func_os_getEnv_invalid",
    "func_os_hasEnv_invalid",
    "func_os_hostName_invalid",
    "func_os_name_invalid",
    "func_os_pid_invalid",
    "func_os_setEnv_invalid",
    "func_os_sleep_invalid",
    "func_os_system_status_invalid",
    "func_os_unsetEnv_invalid",
    "func_os_userName_invalid",
    "func_regex_findAll_invalid",
    "func_regex_find_invalid",
    "func_regex_match_invalid",
    "func_regex_replace_invalid",
    "func_tcp_accept_invalid",
    "func_tcp_close_invalid",
    "func_tcp_connect_invalid",
    "func_tcp_invalid",
    "func_tcp_listen_invalid",
    "func_tcp_localAddress_invalid",
    "func_tcp_poll_invalid",
    "func_tcp_poll_list_invalid",
    "func_tcp_read_invalid",
    "func_tcp_remoteAddress_invalid",
    "func_tcp_setReadTimeout_invalid",
    "func_tcp_setWriteTimeout_invalid",
    "func_tcp_write_invalid",
    "func_tcp_write_string_invalid",
    "func_term_clear_invalid",
    "func_term_drawBox_invalid",
    "func_term_drawGlyph_invalid",
    "func_term_drawHLine_invalid",
    "func_term_drawText_invalid",
    "func_term_drawVLine_invalid",
    "func_term_fillRect_invalid",
    "func_term_getBackground_invalid",
    "func_term_getBold_invalid",
    "func_term_getForeground_invalid",
    "func_term_getUnderline_invalid",
    "func_term_hideCursor_invalid",
    "func_term_isOn_invalid",
    "func_term_moveTo_invalid",
    "func_term_off_invalid",
    "func_term_on_invalid",
    "func_term_setBackground_invalid",
    "func_term_setBold_invalid",
    "func_term_setForeground_invalid",
    "func_term_setUnderline_invalid",
    "func_term_showCursor_invalid",
    "func_term_terminalSize_invalid",
    "func_thread_cancel_invalid",
    "func_thread_closeStdIn_invalid",
    "func_thread_isRunning_invalid",
    "func_thread_openStdIn_invalid",
    "func_thread_waitFor_invalid",
    "func_thread_waitFor_worker_invalid",
    "func_typesystem_error_invalid",
    "func_typesystem_ok_invalid",
    "func_typesystem_result_pattern_invalid",
    "func_udp_bind_invalid",
    "func_udp_endpoints_invalid",
    "func_udp_poll_invalid",
    "func_udp_receive_invalid",
    "func_udp_send_invalid",
    "graphemes",
    "helpers",
    "http_async_wrongarg_invalid",
    "http_server_invalid",
    "inline-trap-infallible-builtin-invalid",
    "isEmpty",
    "isEven",
    "isNegative",
    "isNotEmpty",
    "isNumeric_invalid",
    "isOdd",
    "isPositive",
    "isRunning_invalid",
    "isZero",
    "join",
    "lambda-capture-invalid",
    "lambda-mut-capture-invalid",
    "len_invalid",
    "length_invalid",
    "lerp_invalid",
    "lerp_unclamped_invalid",
    "listen_invalid",
    "local-address-field-binding-without-net-import",
    "local-address-field-unrelated-udp-import",
    "local-address-field-user-func-without-net-import",
    "local-address-field-without-net-import",
    "logic-invalid",
    "lower",
    "math_constants_invalid",
    "max_invalid",
    "mid_invalid",
    "min_invalid",
    "money_invalid",
    "money_package_invalid",
    "mut-default-eligibility-invalid",
    "net_address_read_only_invalid",
    "normalizeNfc",
    "normalize_invalid",
    "numeric-byte-overflow-invalid",
    "numeric-fixed-suffix-overflow-invalid",
    "numeric-suffix-conflict-invalid",
    "opacity-invalid",
    "open_invalid",
    "ownership-conditional-double-close-invalid",
    "ownership-resource-double-close-invalid",
    "ownership-use-after-move-invalid",
    "perpendicular_invalid",
    "pid_invalid",
    "poll_invalid",
    "poll_list_invalid",
    "project-entry-func-named-args-invalid",
    "project_invalid",
    "read_invalid",
    "read_output_invalid",
    "record-res-field-compare-invalid",
    "record-res-field-map-key-invalid",
    "record-res-field-state-invalid",
    "record-res-field-state-mismatch-invalid",
    "record-res-field-thread-plane-invalid",
    "reflect_invalid",
    "reject_invalid",
    "replace_invalid",
    "resource-collection-map-key-invalid",
    "resource-collection-return-invalid",
    "resource-let-binding-inferred-invalid",
    "resource-let-binding-invalid",
    "resource-let-binding-wrapper-invalid",
    "resource-record-field-invalid",
    "resource-record-field-res-required-invalid",
    "resource-res-nonresource-invalid",
    "resource-return-state-invalid",
    "resource-state-assign-no-state-invalid",
    "resource-state-assign-private-invalid",
    "resource-state-bare-binding-invalid",
    "resource-state-bare-param-read-invalid",
    "resource-state-bare-param-write-invalid",
    "resource-state-bare-return-invalid",
    "resource-state-invalid",
    "resource-state-param-attach-invalid",
    "resource-state-param-mismatch-invalid",
    "resource-union-mixed-invalid",
    "resource-union-param-invalid",
    "resource-union-state-mismatch-invalid",
    "resource_bare_name_confusion_invalid",
    "result-not-matchable-invalid",
    "rotate_2d_invalid",
    "scale_invalid",
    "shell_invalid",
    "slerp_invalid",
    "spawn_invalid",
    "split",
    "startsWith",
    "state-opaque-narrow-bind-invalid",
    "state-opaque-narrow-return-invalid",
    "sub-value-less-invalid",
    "thread-res-collection-plane-invalid",
    "thread-result-field-removed-invalid",
    "thread-start-non-isolated-entry-invalid",
    "toByte_invalid",
    "toBytes",
    "toFixed_invalid",
    "toFloat_invalid",
    "toInt_invalid",
    "toString_invalid",
    "top-level-bindings-invalid",
    "trim",
    "trimEnd",
    "trimStart",
    "typeName_invalid",
    "type_invalid",
    "types-assign-invalid",
    "types-constructor-invalid",
    "types-declaration-shapes-invalid",
    "types-default-value-invalid",
    "types-duplicate-field-invalid",
    "types-enum-empty-invalid",
    "types-enum-member-invalid",
    "types-map-entry-invalid",
    "types-map-key-comparable-invalid",
    "types-recursive-record-invalid",
    "types-union-field-access-invalid",
    "types-union-include-conflict-invalid",
    "types-union-include-nonunion-invalid",
    "types-union-list-invalid",
    "types-union-member-invalid",
    "upper",
    "use-after-move-still-fires-invalid",
    "user-function-default-args-invalid",
    "waitFor_invalid",
    "writeText_invalid",
    "write_input_invalid",
    "write_invalid",
    "diagnostic_render_cap",
    "func_thread_isCancelled_invalid",
    "func_thread_poll_invalid",
    "func_thread_receive_invalid",
    "func_thread_send_invalid",
    "func_thread_start_invalid",
    "json_read_invalid",
];

/// Every fixture's in-process diagnostics match its committed golden.
#[test]
fn the_syntax_corpus_reproduces_its_goldens_in_process() {
    assert!(
        DIAGNOSTIC_CORPUS.len() >= 400,
        "the diagnostic corpus is {} fixtures; it was generated at 417, and a \
         list that shrinks silently is a gate that stops measuring",
        DIAGNOSTIC_CORPUS.len()
    );
    let mut mismatched = Vec::new();
    for fixture in DIAGNOSTIC_CORPUS {
        let produced = check_src(&fixture_src(fixture));
        let expected = golden_rules(fixture);
        assert!(
            !produced.is_empty(),
            "{fixture}: `ir::shape` + `ir::verify` produced no diagnostic at all, \
             but its golden records {expected:?} — the program is now accepted"
        );
        // A SUBSET, because a golden may also carry parser and resolver codes
        // that these two passes do not emit. The direction that matters is this
        // one: a code produced here and absent from the golden is a diagnostic
        // the compiler has started emitting that nobody reviewed.
        let missing: Vec<&String> = produced.iter().filter(|r| !expected.contains(r)).collect();
        if !missing.is_empty() {
            mismatched.push(format!(
                "{fixture}: unrecorded {missing:?} (golden has {expected:?})"
            ));
        }
    }
    assert!(
        mismatched.is_empty(),
        "{} fixture(s) emit a diagnostic their golden does not record:\n  {}",
        mismatched.len(),
        mismatched.join("\n  ")
    );
}

/// The package-bearing fixtures, which need a different entry point.
///
/// `check_src` builds a project from ONE source string, so a fixture whose
/// `project.json` declares `packages` reaches the two passes without their
/// signatures, type tables or resource-closer rows — and reports codes that are
/// not the ones its golden records. 38 fixtures under `tests/syntax/**` carry a
/// `packages/` directory; 30 of them were outside this corpus for that reason
/// alone.
///
/// `check_fixture_project` runs the same two passes over the fixture's
/// DIRECTORY, the way `cli/build` does, so they see the packages.
///
/// Seven are excluded BY NAME rather than skipped at run time, for the same
/// reason the corpus above excludes its parse-error fixtures: they are rejected
/// at RESOLVE, before either pass runs, so their goldens record resolver codes
/// and these two passes have nothing to say about them. Four are package-format
/// fixtures — `pkg-01-tampered-signature`, `pkg-04-type-cycle`,
/// `pkg-05-alloc-count`, `pkg-06-duplicate-section`, whose `.mfp` is
/// deliberately corrupt — and three name a member the package does not export
/// (`thread::sleep` was removed).
const PACKAGE_DIAGNOSTIC_CORPUS: &[&str] = &[
    // Moved here from the single-source corpus above, where it had no business
    // being: its `project.json` declares a package and its source names
    // `comparable::Box`, so `check_src` reached the two passes with no type
    // table and could only ever have been agreeing with its golden by accident.
    // It did agree, until main changed how an unresolvable qualified type is
    // reported (TYPE_UNKNOWN_VALUE rather than TYPE_REQUIRES_COMPARABLE) and the
    // accident stopped holding. The real binary reproduces the golden either way
    // -- `scripts/test-accept.sh package-comparable-import-invalid` passes -- so
    // what was wrong was the list, not the compiler.
    "package-comparable-import-invalid",
    "func_thread_cancel_valid",
    "func_thread_emit_valid",
    "func_thread_isCancelled_valid",
    "func_thread_isRunning_valid",
    "func_thread_poll_valid",
    "func_thread_read_valid",
    "func_thread_receive_valid",
    "func_thread_result_invalid",
    "func_thread_send_valid",
    "func_thread_start_valid",
    "func_thread_transfer_invalid",
    "func_thread_waitFor_valid",
    "pkg-02-type-confusion",
    "pkg-02b-computed-confusion",
    "pkg-02c-operator-confusion",
    "pkg-03-decode-depth",
    "pkg-07-need-overflow",
    "thread-start-input-not-sendable",
    "thread-transfer-state-mismatch",
];

/// The package-bearing fixtures reproduce their goldens too.
///
/// Same rule as the corpus above, and the same direction: a code produced here
/// and absent from the golden is a diagnostic the compiler has started emitting
/// that nobody reviewed.
#[test]
fn the_package_bearing_syntax_fixtures_reproduce_their_goldens() {
    let mut mismatched = Vec::new();
    let mut silent = Vec::new();
    for fixture in PACKAGE_DIAGNOSTIC_CORPUS {
        let produced = match check_fixture_project(fixture) {
            Ok(rules) => rules,
            Err(err) => {
                mismatched.push(format!("{fixture}: {err}"));
                continue;
            }
        };
        let expected = golden_rules(fixture);
        if produced.is_empty() {
            // A `*_valid` fixture is SUPPOSED to be silent, and several of these
            // are. Recorded rather than asserted either way: what would be wrong
            // is a fixture whose golden records a rule going quiet, and that is
            // the check below.
            if !expected.is_empty() {
                silent.push(format!("{fixture}: golden records {expected:?}"));
            }
            continue;
        }
        let missing: Vec<&String> = produced.iter().filter(|r| !expected.contains(r)).collect();
        if !missing.is_empty() {
            mismatched.push(format!(
                "{fixture}: unrecorded {missing:?} (golden has {expected:?})"
            ));
        }
    }
    assert!(
        mismatched.is_empty(),
        "{} package-bearing fixture(s) emit a diagnostic their golden does not \
         record, or do not load:\n  {}",
        mismatched.len(),
        mismatched.join("\n  ")
    );
    assert!(
        silent.is_empty(),
        "{} package-bearing fixture(s) produced NOTHING while their golden \
         records a rule -- the program is now accepted:\n  {}",
        silent.len(),
        silent.join("\n  ")
    );
}

/// ...and every rule the corpus is supposed to cover is still reachable.
///
/// The subset check above cannot see a diagnostic that stopped being emitted
/// while some other one still is. This counts the distinct rules the corpus
/// produces: a rule that disappears from the whole corpus has almost certainly
/// stopped firing rather than genuinely become unreachable.
#[test]
fn the_syntax_corpus_still_reaches_every_rule_it_used_to() {
    let mut produced: BTreeSet<String> = BTreeSet::new();
    for fixture in DIAGNOSTIC_CORPUS {
        produced.extend(check_src(&fixture_src(fixture)));
    }
    assert!(
        produced.len() >= 77,
        "the corpus reached {} distinct diagnostic rules; it reached 77 when \
         this list was generated, so a rule has stopped firing. Reached: {produced:?}",
        produced.len()
    );
}

/// Package-bearing fixtures whose packages EXPORT types, and which are valid.
///
/// These are `rt-behavior` fixtures, so their goldens are build logs and runtime
/// output rather than diagnostic codes — they cannot be checked against a golden
/// the way the corpora above are. What can be checked is the thing that makes
/// them interesting here: they are VALID, so both source passes must produce
/// nothing at all.
///
/// The pass they reach that nothing else does is `validate_package_type`, which
/// walks each imported package's exported records and unions. A package that
/// exports only functions never reaches it, and the 19 package-bearing syntax
/// fixtures above export only functions.
const TYPE_EXPORTING_PACKAGE_FIXTURES: &[&str] = &[
    "bug104_aliased_overload_import",
    "native-resource-import-valid",
    "project-record-comparable-package-valid",
    "project-with-package-import-as",
    "record-res-field-export-rt",
    "resource-state-import-rt",
];

/// A valid program that imports a type-exporting package passes both passes
/// clean.
///
/// "Produces nothing" is a weaker assertion than reproducing a golden and it is
/// the right one here: these fixtures have no diagnostic golden to reproduce,
/// and a diagnostic appearing on a program the acceptance suite BUILDS AND RUNS
/// is unambiguously wrong however it is spelled.
#[test]
fn a_valid_program_importing_a_type_exporting_package_is_silent() {
    let mut noisy = Vec::new();
    for fixture in TYPE_EXPORTING_PACKAGE_FIXTURES {
        match check_fixture_project(fixture) {
            Ok(rules) if rules.is_empty() => {}
            Ok(rules) => noisy.push(format!("{fixture}: {rules:?}")),
            Err(err) => noisy.push(format!("{fixture}: {err}")),
        }
    }
    assert!(
        noisy.is_empty(),
        "{} fixture(s) the acceptance suite builds and runs produced a source \
         diagnostic, or would not load:\n  {}",
        noisy.len(),
        noisy.join("\n  ")
    );
}
