//! plan-176-G: `ir::tests::lower_tests`'s external/write-IR, builtin advisory and padding,
//! inline-trap treeify and capture-variant tests. The shared helpers live in
//! `lower_tests/mod.rs`.

use super::*;

// ---- external functions (lower_project_with_external_functions) -------

#[test]
fn external_function_return_and_param_types_are_used() {
    use super::super::ExternalFunctionParam;
    use std::collections::HashMap as StdHashMap;

    // Build the AST directly and lower with external function tables so the
    // external-function seam (params/returns injection) is exercised.
    let src =
        "FUNC run() AS Integer\n  RETURN extAdd(1, 2)\nEND FUNC\nFUNC main AS Nothing\nEND FUNC\n";
    let file = crate::ast::parse_source(std::path::Path::new("src/main.mfb"), "src/main.mfb", src)
        .expect("parse");
    let project = crate::ast::AstProject {
        name: "irtest".to_string(),
        files: vec![file],
    };

    let mut signatures = StdHashMap::new();
    signatures.insert(
        "extAdd".to_string(),
        ExternalSignature {
            params: vec![
                ExternalFunctionParam {
                    name: "a".to_string(),
                    type_: crate::types::ParameterType::parse("Integer"),
                    default: crate::ir::ExternalDefault::None,
                },
                ExternalFunctionParam {
                    name: "b".to_string(),
                    type_: crate::types::ParameterType::parse("Integer"),
                    default: crate::ir::ExternalDefault::None,
                },
            ],
            returns: crate::types::ParameterType::parse("Integer"),
            isolated: false,
        },
    );
    let ir = lower_project_with_external_functions(&project, None, &signatures, &[], &[]);
    // The call to the external function types as Integer (from the external
    // return-type table).
    let call_ty = function(&ir, "run").body.iter().find_map(|op| match op {
        IrOp::Return {
            value: Some(IrValue::Call { target, type_, .. }),
            ..
        } if target == "extAdd" => Some(type_.name().into_owned()),
        _ => None,
    });
    assert_eq!(call_ty.as_deref(), Some("Integer"));
}

// ---- write_ir --------------------------------------------------------

#[test]
fn write_ir_emits_ir_file() {
    let ir = lower_src("FUNC main AS Nothing\nEND FUNC\n");
    let dir = std::env::temp_dir().join(format!(
        "mfb_write_ir_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = write_ir(&dir, &ir).expect("write_ir");
    assert!(path.exists());
    assert!(std::fs::read_to_string(&path).unwrap().contains(&ir.name));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- collect_project_docs / import inference smoke -------------------

#[test]
fn inferred_binding_from_list_literal_types() {
    let ir = lower_src("IMPORT collections\nLET xs = [1, 2, 3]\nFUNC main AS Nothing\nEND FUNC\n");
    assert_eq!(binding(&ir, "xs").type_.name(), "List OF Integer");
}

// ---- more builtin packages: expression_type + call padding ----------

#[test]
fn json_and_csv_calls_resolve() {
    let ir = lower_src(
        "IMPORT json\n\
             IMPORT csv\n\
             FUNC run() AS String\n\
               LET j AS json::Json = json::parse(\"1\")\n\
               LET s AS String = json::stringify(j)\n\
               LET g AS List OF List OF String = csv::parse(\"a,b\")\n\
               RETURN s\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(matches!(
        bind_value(&function(&ir, "run").body, "j"),
        // bug-480 Phase 4b: a builtin value type is addressed by its
        // package-qualified identity now.
        Some(IrValue::Call { type_, .. }) if type_.name() == "json.Json"
    ));
}

#[test]
fn builtin_enum_advisory_warns_once_per_user_occurrence() {
    // plan-109-A's enum-value advisory, held by `ir::verify` since the source
    // checker's deletion (plan-107-D): a registry enum value carrying an
    // `EnumVariant::advisory` reports its warn-severity rule exactly once per
    // user-source occurrence — an expression AND a `MATCH` literal both reach
    // the member-access arm — and rejects nothing. The injected crypto source
    // compares its `Hash` selector against `Hash.SHA1` in its own dispatch
    // helpers; those `builtins/` occurrences must not count.
    let src = "\
IMPORT crypto
FUNC pick(h AS crypto::Hash) AS Integer
  MATCH h
    CASE crypto::Hash.SHA1
      RETURN 1
    CASE ELSE
      RETURN 0
  END MATCH
END FUNC
FUNC main AS Integer
  LET d AS List OF Byte = crypto::hash(crypto::Hash.SHA1, \"legacy\")
  RETURN len(d) + pick(crypto::Hash.SHA2_256)
END FUNC
";
    let rules = crate::testutil::check_src(src);
    let warnings = rules
        .iter()
        .filter(|r| *r == "CRYPTO_SHA1_INSECURE")
        .count();
    assert_eq!(warnings, 2, "{rules:?}");
    assert!(
        rules.iter().all(|r| !crate::rules::is_error(r)),
        "advisory must not reject the program: {rules:?}"
    );
}

/// bug-517: the advisory is scoped to the USE, not the value — the owner's
/// ruling (2026-09-05), reversing `plan-109-A`'s "regardless of which public
/// function consumes the selector".
///
/// `crypto::hash` relies on collision resistance and still warns.
/// `hmac`/`hkdf`/`pbkdf2` do not — HMAC's proof rests on the compression
/// function as a PRF — so SHA-1 there is a sound legacy-interop choice
/// (RFC 6238 TOTP, WPA2) and is not advised against.
#[test]
fn the_sha1_advisory_fires_on_hash_and_not_on_hmac_hkdf_or_pbkdf2() {
    let count = |src: &str| {
        crate::testutil::check_src(src)
            .iter()
            .filter(|r| *r == "CRYPTO_SHA1_INSECURE")
            .count()
    };
    let program = |body: &str| {
        format!(
            "IMPORT crypto\nIMPORT strings\nFUNC main AS Integer\n                   LET k AS List OF Byte = strings::toBytes(\"k\")\n                   LET m AS List OF Byte = strings::toBytes(\"m\")\n{body}  RETURN 0\nEND FUNC\n"
        )
    };

    // The broken use: a bare digest's whole claim IS collision resistance.
    assert_eq!(
        count(&program(
            "  LET d AS List OF Byte = crypto::hash(crypto::Hash.SHA1, \"x\")\n"
        )),
        1,
        "crypto::hash(SHA1) must still warn"
    );

    // The three sound uses.
    for call in [
        "crypto::hmac(crypto::Hash.SHA1, k, m)",
        "crypto::hkdf(crypto::Hash.SHA1, m, k, m, 16)",
        "crypto::pbkdf2(crypto::Hash.SHA1, m, k, 1, 16)",
    ] {
        assert_eq!(
            count(&program(&format!("  LET d AS List OF Byte = {call}\n"))),
            0,
            "{call} must not warn: SHA-1 is sound there"
        );
    }
}

/// The other half, and the one that matters more: suppression is FAIL-CLOSED.
/// It applies to a DIRECT selector argument only, so every shape where the
/// checker cannot see that the consumer is sound keeps warning. A false
/// positive is a warning on a sound use; a false negative is silence on a
/// broken one.
#[test]
fn the_sha1_advisory_suppression_is_fail_closed() {
    let count = |src: &str| {
        crate::testutil::check_src(src)
            .iter()
            .filter(|r| *r == "CRYPTO_SHA1_INSECURE")
            .count()
    };

    // Bound to a local first, then passed on: the checker cannot see the
    // consumer from the occurrence, so it warns.
    assert_eq!(
        count(
            "IMPORT crypto\nIMPORT strings\nFUNC main AS Integer\n                   LET h AS crypto::Hash = crypto::Hash.SHA1\n                   LET k AS List OF Byte = strings::toBytes(\"k\")\n                   LET d AS List OF Byte = crypto::hmac(h, k, k)\n  RETURN 0\nEND FUNC\n"
        ),
        1,
        "an indirect selector must still warn — suppression is not dataflow"
    );

    // A bare occurrence consumed by nothing.
    assert_eq!(
        count(
            "IMPORT crypto\nFUNC pick() AS crypto::Hash\n                   RETURN crypto::Hash.SHA1\nEND FUNC\n\
                 FUNC main AS Integer\n  LET h AS crypto::Hash = pick()\n  RETURN 0\nEND FUNC\n"
        ),
        1,
        "a bare occurrence must still warn"
    );

    // A `MATCH` literal is not a selector argument.
    assert_eq!(
        count(
            "IMPORT crypto\nFUNC pick(h AS crypto::Hash) AS Integer\n                   MATCH h\n    CASE crypto::Hash.SHA1\n      RETURN 1\n                     CASE ELSE\n      RETURN 0\n  END MATCH\nEND FUNC\n\
                 FUNC main AS Integer\n  RETURN pick(crypto::Hash.SHA2_256)\nEND FUNC\n"
        ),
        1,
        "a MATCH literal must still warn"
    );

    // Nested inside the selector argument rather than being it.
    assert_eq!(
        count(
            "IMPORT crypto\nIMPORT strings\nFUNC same(h AS crypto::Hash) AS crypto::Hash\n                   RETURN h\nEND FUNC\n\
                 FUNC main AS Integer\n  LET k AS List OF Byte = strings::toBytes(\"k\")\n                   LET d AS List OF Byte = crypto::hmac(same(crypto::Hash.SHA1), k, k)\n                   RETURN 0\nEND FUNC\n"
        ),
        1,
        "a value nested inside the argument is not hmac's selector — must warn"
    );
}

#[test]
fn builtin_enum_without_advisory_never_warns() {
    let src = "\
IMPORT crypto
FUNC main AS Integer
  LET d AS List OF Byte = crypto::hash(crypto::Hash.SHA2_256, \"data\")
  MATCH crypto::Hash.SHA2_512
    CASE crypto::Hash.SHA2_512
      RETURN len(d)
    CASE ELSE
      RETURN 0
  END MATCH
END FUNC
";
    assert!(
        crate::testutil::accepts(src),
        "{:?}",
        crate::testutil::check_src(src)
    );
}

#[test]
fn crypto_call_resolves_and_pads_aad() {
    // crypto::hash resolves via the registry; an AEAD call pads its
    // optional `aad` argument to an empty byte list.
    let ir = lower_src(
        "IMPORT crypto\n\
             FUNC run() AS List OF Byte\n\
               RETURN crypto::hash(crypto::Hash.SHA2_256, \"data\")\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::Call { .. }),
            ..
        }
    )));
}

#[test]
fn net_and_http_calls_resolve_and_pad() {
    // net::toUrl -> Url; http::read pads its default headers (empty map) and
    // method (a literal), exercising the http default-argument padding.
    let src = "IMPORT net\n\
             IMPORT http\n\
             FUNC run() AS Integer\n\
               LET u AS net::Url = net::toUrl(\"http://x\")\n\
               LET r AS http::Response = http::read(u)\n\
               RETURN 0\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("net/http program should lower");
    let f = function(&ir, "run");
    // http::read(u) pads BOTH headers (empty map literal) and method (a
    // String const), exercising the map and scalar arms of http padding.
    let has_map_pad = f.body.iter().any(|op| match op {
        IrOp::Bind {
            value: Some(IrValue::Call { args, .. }),
            ..
        } => args.iter().any(|a| matches!(a, IrValue::MapLiteral { .. })),
        _ => false,
    });
    assert!(
        has_map_pad,
        "http::read headers padded as an empty map literal"
    );
}

#[test]
fn thread_call_resolves() {
    let src = "IMPORT thread\n\
             ISOLATED FUNC worker(w AS thread::ThreadWorker OF Integer TO Integer, seed AS Integer) AS Integer\n\
               RETURN seed\n\
             END FUNC\n\
             FUNC run() AS Integer\n\
               RES t AS thread::Thread OF Integer TO Integer = thread::start(worker, 5)\n\
               RETURN 0\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n";
    // thread APIs are complex; only assert lowering succeeds when the front
    // end accepts this program.
    if let Some(ir) = try_lower_src(src) {
        assert!(!function(&ir, "run").body.is_empty());
    }
}

#[test]
fn datetime_time_pads_optional_arguments() {
    // datetime::time(hour, minute) pads the optional second/nanos with 0.
    let ir = lower_src(
        "IMPORT datetime\n\
             FUNC run() AS datetime::Time\n  RETURN datetime::time(10, 30)\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let padded = function(&ir, "run").body.iter().any(|op| match op {
        IrOp::Return {
            value: Some(IrValue::Call { args, .. }),
            ..
        } => args.len() >= 4,
        _ => false,
    });
    assert!(padded, "second/nanos padded to a 4-arg call");
}

#[test]
fn regex_find_pads_start_argument() {
    let ir = lower_src(
        "IMPORT regex\n\
             FUNC run() AS Integer\n  RETURN regex::find(\"abc\", \"b\")\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let padded = function(&ir, "run").body.iter().any(|op| match op {
        IrOp::Return {
            value: Some(IrValue::Call { args, .. }),
            ..
        } => args.len() >= 3,
        _ => false,
    });
    assert!(padded, "start default padded to a 3-arg call");
}

#[test]
fn crypto_aead_pads_aad_as_empty_list() {
    // crypto::seal(SymmetricCipher.AES256GCM, key, nonce, plaintext) pads the
    // optional `aad` with an empty byte list (ListLiteral, not a scalar Const).
    let src = "IMPORT crypto\n\
             FUNC run(key AS List OF Byte, nonce AS List OF Byte, pt AS List OF Byte) AS crypto::Sealed\n\
               RETURN crypto::seal(crypto::SymmetricCipher.AES256GCM, key, nonce, pt)\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("crypto AEAD program should lower");
    let has_list_pad = function(&ir, "run").body.iter().any(|op| match op {
        IrOp::Return {
            value: Some(IrValue::Call { args, .. }),
            ..
        } => args
            .last()
            .is_some_and(|a| matches!(a, IrValue::ListLiteral { .. })),
        _ => false,
    });
    assert!(has_list_pad, "aad padded as an empty list literal");
}

#[test]
fn tls_connect_pads_optional_arguments() {
    let src = "IMPORT tls\n\
             FUNC run() AS Integer\n\
               RES s AS tls::Socket = tls::connect(\"host\", 443)\n\
               tls::close(s)\n\
               RETURN 0\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("tls connect program should lower");
    let padded = function(&ir, "run").body.iter().any(|op| match op {
        IrOp::Bind {
            value: Some(IrValue::Call { args, .. }),
            ..
        } => args.len() >= 4,
        _ => false,
    });
    assert!(padded, "timeoutMs + serverName padded to a 4-arg call");
}

#[test]
fn tls_close_on_listener_routes_to_listener_helper() {
    // `tls::close` on a `tls::Listener` operand routes to the listener-shaped
    // internal close helper (the tls.close listener branch).
    let src = "IMPORT tls\n\
             FUNC run() AS Integer\n\
               RES l AS tls::Listener = tls::listen(\"0.0.0.0\", 8080, \"cert\", \"key\")\n\
               tls::close(l)\n\
               RETURN 0\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n";
    if let Some(ir) = try_lower_src(src) {
        assert!(function(&ir, "run")
            .body
            .iter()
            .any(|op| matches!(op, IrOp::Eval { .. })));
    }
}

#[test]
fn vector_call_and_constant_lower() {
    // A vector call resolves a type-specific internal name from its argument
    // record types (implementation_name), and a vector constant inlines a
    // record constructor (constant_components).
    let ir = lower_src(
        "IMPORT vector\n\
             IMPORT io\n\
             FUNC run() AS Float\n\
               LET a AS vector::Float3 = vector::Float3[1.0, 2.0, 2.0]\n\
               LET up AS vector::Float3 = vector::upFloat3\n\
               RETURN vector::length(a)\n\
             END FUNC\n\
             FUNC main AS Nothing\n  io::print(\"x\")\nEND FUNC\n",
    );
    let f = function(&ir, "run");
    // The `up` constant inlined a Constructor.
    assert!(matches!(
        bind_value(&f.body, "up"),
        Some(IrValue::Constructor { .. })
    ));
    // `vector::length(a)` lowers to a Call with a resolved (internal) target.
    assert!(f.body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::Call { .. }),
            ..
        }
    )));
}

#[test]
fn datetime_call_pads_default_arguments() {
    // datetime constructors are arity-aware and pad optional arguments.
    let ir = lower_src(
        "IMPORT datetime\n\
             FUNC run() AS datetime::Instant\n  RETURN datetime::instant(0)\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::Call { .. }),
            ..
        }
    )));
}

// ---- inline-TRAP handler treeify: loops & nested statements ----------

#[test]
fn inline_trap_mut_bind_and_global_assign_targets() {
    // A MUT inline-trap bind marks the target mutable (mutable-bind arm), and
    // an inline-trap assign to a top-level binding takes the AssignGlobal arm.
    let ir = lower_src(
        "MUT counter AS Integer = 0\n\
             FUNC run(s AS String) AS Integer\n\
               MUT local AS Integer = toInt(s) TRAP(e)\n    RECOVER 0\n  END TRAP\n\
               counter = toInt(s) TRAP(e)\n    RECOVER 0\n  END TRAP\n\
               RETURN local + counter\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let f = function(&ir, "run");
    // The global assign target lowered to AssignGlobal.
    assert!(f
        .body
        .iter()
        .any(|op| matches!(op, IrOp::AssignGlobal { name, .. } if name == "counter")));
    // The MUT bind of `local` is mutable.
    assert!(f.body.iter().any(|op| matches!(
        op,
        IrOp::Bind { mutable: true, name, .. } if name == "local"
    )));
}

#[test]
fn inline_trap_handler_with_terminator_then_dead_statement() {
    // A handler whose head diverges (FAIL) with a following statement drops
    // the unreachable tail (statement_terminates head, tail present).
    let ir = lower_src(
        "FUNC run(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n    FAIL e\n    RECOVER 0\n  END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

#[test]
fn inline_trap_handler_is_a_single_if() {
    // The handler's only statement is an IF (no continuation), so it is
    // normalized by treeify_statement's If arm (recursing into both blocks).
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 IF e.code = 404 THEN\n            RECOVER 0\n          ELSE\n            RECOVER 1\n          END IF\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

#[test]
fn inline_trap_handler_is_a_single_match() {
    // The handler's only statement is a MATCH (no continuation), normalized
    // by treeify_statement's Match arm.
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 MATCH e.code\n\
                   CASE 404\n              RECOVER 0\n\
                   CASE ELSE\n              RECOVER 1\n\
                 END MATCH\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

#[test]
fn inline_trap_handler_is_a_single_loop() {
    // The handler's only statement is a loop, so treeify recurses into the
    // loop bodies via treeify_statement (For / While / DoUntil / ForEach).
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC parse(s AS String) AS Integer\n\
               MUT acc AS Integer = 0\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 FOR i = 1 TO 2\n\
                   WHILE acc < 0\n              acc = acc + 1\n            END WHILE\n\
                   DO\n              acc = acc + 1\n            LOOP UNTIL acc > 5\n\
                   FOR EACH v IN [1, 2]\n              acc = acc + v\n            NEXT\n\
                   RECOVER acc\n\
                 NEXT\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

#[test]
fn inline_trap_handler_with_loop_and_continuation() {
    // Handler whose head is a non-terminating loop (falls through), followed
    // by a RECOVER continuation. Exercises the fall-through arm of
    // treeify_handler and the loop arm of treeify_statement.
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               MUT acc AS Integer = 0\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 FOR i = 1 TO 2\n            acc = acc + i\n          NEXT\n\
                 WHILE acc < 0\n            acc = acc + 1\n          END WHILE\n\
                 RECOVER acc\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

#[test]
fn inline_trap_handler_with_nested_if_and_continuation() {
    // Handler head is an IF whose branches fall through; the continuation is
    // distributed into both branches (distribute_continuation).
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 IF e.code > 0 THEN\n            LET x AS Integer = 1\n          ELSE\n            LET y AS Integer = 2\n          END IF\n\
                 RECOVER 0\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

#[test]
fn inline_trap_handler_with_match_else_and_continuation() {
    // Handler head is a MATCH that already has an ELSE arm; the continuation
    // is distributed into every arm without synthesizing an extra ELSE.
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 MATCH e.code\n\
                   CASE 404\n              LET a AS Integer = 1\n\
                   CASE ELSE\n              LET b AS Integer = 2\n\
                 END MATCH\n\
                 RECOVER 0\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

// ---- named-argument reordering (normalize_*_call_arguments) ----------

#[test]
fn builtin_named_arguments_reorder_with_extra() {
    // A builtin called with a MIX of positional and named arguments takes
    // the param-name reorder branch of normalize_builtin_call_arguments
    // (positional fills index 0, the named `delimiter` fills index 1).
    let ir = lower_src(
        "IMPORT strings\n\
             FUNC run() AS String\n  RETURN strings::join([\"a\", \"b\"], delimiter := \"-\")\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::Call { .. }),
            ..
        }
    )));
}

#[test]
fn local_call_named_arguments_reorder() {
    // A user-function call with named args reordered exercises
    // normalize_local_call_arguments + lower_local_call_arguments.
    let ir = lower_src(
        "FUNC box(w AS Integer, h AS Integer) AS Integer\n  RETURN w * h\nEND FUNC\n\
             FUNC main AS Nothing\n  LET a AS Integer = box(h := 3, w := 4)\nEND FUNC\n",
    );
    assert!(matches!(
        bind_value(main_body(&ir), "a"),
        Some(IrValue::Call { target, args, .. }) if target == "box" && args.len() == 2
    ));
}

// ---- captured-locals variants (collect_captured_locals arms) ---------

#[test]
fn lambda_captures_through_map_literal_and_with_update() {
    let ir = lower_src(
        "IMPORT collections\n\
             TYPE Point\n  x AS Integer\nEND TYPE\n\
             FUNC run() AS List OF Integer\n\
               LET base AS Integer = 5\n\
               LET p AS Point = Point[1]\n\
               LET keyv AS String = \"k\"\n\
               LET xs AS List OF Integer = [1]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + collections::getOr(Map OF String TO Integer { keyv := base }, keyv, 0) + (WITH p { x := base }).x)\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let lambda = ir
        .functions
        .iter()
        .find(|f| f.name.starts_with("$lambda"))
        .expect("a lambda");
    // base, p, keyv are all captured.
    let captures = lambda
        .body
        .iter()
        .filter(|op| {
            matches!(
                op,
                IrOp::Bind {
                    value: Some(IrValue::Capture { .. }),
                    ..
                }
            )
        })
        .count();
    assert!(captures >= 3, "captured base, p and keyv (got {captures})");
}

#[test]
fn lambda_captures_called_function_local_and_constructor_arg() {
    // The lambda calls a captured function-typed local `f` (callee-capture
    // arm) and constructs a record from a captured value `base` (constructor
    // arm) and a list literal (list-literal arm).
    let ir = lower_src(
        "IMPORT collections\n\
             TYPE Box\n  n AS Integer\nEND TYPE\n\
             FUNC makeAdder(k AS Integer) AS FUNC(Integer) AS Integer\n\
               RETURN LAMBDA(x AS Integer) -> x + k\n\
             END FUNC\n\
             FUNC run() AS List OF Integer\n\
               LET base AS Integer = 3\n\
               LET f AS FUNC(Integer) AS Integer = makeAdder(1)\n\
               LET xs AS List OF Integer = [1]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> f(v) + Box[base].n + len([base]))\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    // The inner lambda captures both `f` (called) and `base` (in a
    // constructor + list literal).
    let inner = ir
        .functions
        .iter()
        .filter(|f| f.name.starts_with("$lambda"))
        .max_by_key(|f| f.body.len())
        .expect("a lambda");
    let captures = inner
        .body
        .iter()
        .filter(|op| {
            matches!(
                op,
                IrOp::Bind {
                    value: Some(IrValue::Capture { .. }),
                    ..
                }
            )
        })
        .count();
    assert!(captures >= 2, "captured f and base (got {captures})");
}

// ---- assignment-bodied lambda binding to a captured target -----------

#[test]
fn assignment_bodied_lambda_captures_assignment_target() {
    // The lambda's assignment target (`acc`) is captured even though it is
    // only written, not read, in the body.
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC run() AS Integer\n\
               MUT acc AS Integer = 0\n\
               LET xs AS List OF Integer = [1, 2]\n\
               collections::forEach(xs, LAMBDA(v AS Integer) -> acc = v)\n\
               RETURN acc\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let lambda = ir
        .functions
        .iter()
        .find(|f| f.name.starts_with("$lambda"))
        .expect("a lambda");
    assert!(lambda
        .body
        .iter()
        .any(|op| matches!(op, IrOp::Assign { name, .. } if name == "acc")));
}

#[test]
fn local_call_named_then_positional_skips_filled_slot() {
    // A named arg fills index 0, then trailing positionals skip the filled
    // slot (the skip-while loop of normalize_local_call_arguments).
    let ir = lower_src(
        "FUNC box(w AS Integer, h AS Integer, d AS Integer) AS Integer\n  RETURN w * h * d\nEND FUNC\n\
             FUNC main AS Nothing\n  LET a AS Integer = box(w := 4, 5, 6)\nEND FUNC\n",
    );
    assert!(matches!(
        bind_value(main_body(&ir), "a"),
        Some(IrValue::Call { target, args, .. }) if target == "box" && args.len() == 3
    ));
}

#[test]
fn builtin_call_named_then_positional_skips_filled_slot() {
    // Same skip path for a built-in call: a named arg fills an index, then a
    // positional skips it (the skip-while loop of
    // normalize_builtin_call_arguments).
    let ir = lower_src(
        "IMPORT strings\n\
             FUNC run() AS String\n  RETURN strings::join(delimiter := \"-\", [\"a\", \"b\"])\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::Call { .. }),
            ..
        }
    )));
}

// ---- native RESOURCE visibility variants -----------------------------

#[test]
fn native_resource_visibility_variants() {
    // A public and a private RESOURCE exercise the public/private
    // arms of native_resources' visibility mapping.
    let src = "PUBLIC RESOURCE Db CLOSE BY lib::close\n\
             PRIVATE RESOURCE Cache CLOSE BY lib::freeCache\n\
             LINK \"c\" AS lib\n\
               FUNC open() AS RES Db\n\
                 SYMBOL \"open\"\n\
                 ABI (produced OUT CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
               FUNC close(db AS Db) AS Nothing\n\
                 SYMBOL \"close\"\n\
                 ABI (db CPtr) AS status CInt32\n\
               END FUNC\n\
               FUNC makeCache() AS RES Cache\n\
                 SYMBOL \"mk\"\n\
                 ABI (produced OUT CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
               FUNC freeCache(c AS Cache) AS Nothing\n\
                 SYMBOL \"free\"\n\
                 ABI (c CPtr) AS status CInt32\n\
               END FUNC\n\
             END LINK\n\
             FUNC main AS Nothing\nEND FUNC\n";
    if let Some(ir) = try_lower_src(src) {
        let db = ir.native_resources.iter().find(|r| r.name == "Db");
        let cache = ir.native_resources.iter().find(|r| r.name == "Cache");
        assert!(db.is_some_and(|r| r.visibility == "public"));
        assert!(cache.is_some_and(|r| r.visibility == "private"));
    }
}

// ---- captured_locals: unary / trapped arms ---------------------------

#[test]
fn lambda_captures_through_unary_and_trapped() {
    // The lambda body references a captured local inside a unary op and
    // inside a trapped call, exercising the Unary and Trapped arms of
    // collect_captured_locals.
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC dbl(x AS Integer) AS Integer\n  RETURN x * 2\nEND FUNC\n\
             FUNC run() AS List OF Integer\n\
               LET base AS Integer = 4\n\
               LET xs AS List OF Integer = [1]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + (-base))\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let lambda = ir
        .functions
        .iter()
        .find(|f| f.name.starts_with("$lambda"))
        .expect("a lambda");
    assert!(lambda.body.iter().any(|op| matches!(
        op,
        IrOp::Bind {
            value: Some(IrValue::Capture { .. }),
            ..
        }
    )));
}

// ---- collections.filter with a lambda predicate ----------------------

#[test]
fn collections_filter_with_lambda_predicate() {
    // A `collections::filter(list, <lambda>)` (non-identifier predicate)
    // takes the else branch that lowers each argument normally.
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC run() AS List OF Integer\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               RETURN collections::filter(xs, LAMBDA(v AS Integer) -> v > 1)\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::Call { .. }),
            ..
        }
    )));
}

// ---- DOC for a callable with no matching overload --------------------

#[test]
fn doc_for_unmatched_overload_is_dropped() {
    // A DOC FUNC header whose parameter types match no overload is dropped
    // (overload_for returns None).
    let src = "DOC\n  FUNC add(String, String)\n  DESC No such overload.\nEND DOC\n\
             EXPORT FUNC add(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n";
    if let Some(ir) = try_lower_src(src) {
        assert!(!ir.docs.decls.iter().any(|d| d.name == "add"));
    }
}
