//! plan-176-G: `ir::tests::lower_tests`'s inline-trap, union-wrapping, native-link, doc
//! and state, and builtin-call/capture tests. The shared helpers live in `lower_tests/mod.rs`.

use super::*;

// ---- inline TRAP -----------------------------------------------------

#[test]
fn inline_trap_bind_with_recover() {
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n    RECOVER 0\n  END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let f = function(&ir, "parse");
    assert!(f.body.iter().any(|op| matches!(
        op,
        IrOp::Bind {
            value: Some(IrValue::CallResult { .. }),
            ..
        }
    )));
    assert!(f.body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

#[test]
fn inline_trap_assign_target() {
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               MUT n AS Integer = 100\n\
               n = toInt(s) TRAP(e)\n    RECOVER 0\n  END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::Bind {
            value: Some(IrValue::CallResult { .. }),
            ..
        }
    )));
}

#[test]
fn inline_trap_discard_bare_statement() {
    let ir = lower_src(
        "FUNC effect(x AS Integer) AS Nothing\n  IF x < 0 THEN FAIL error(1, \"neg\")\nEND FUNC\n\
             FUNC run() AS Nothing\n\
               effect(-1) TRAP(e)\n    RECOVER\n  END TRAP\n\
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
fn inline_trap_handler_with_if_then_fail() {
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 IF e.code = 404 THEN RECOVER 0\n\
                 FAIL e\n\
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
fn inline_trap_handler_with_match_and_continuation() {
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 MATCH e.code\n\
                   CASE 404\n          RECOVER 0\n\
                 END MATCH\n\
                 FAIL e\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    fn has_match(ops: &[IrOp]) -> bool {
        ops.iter().any(|op| match op {
            IrOp::Match { .. } => true,
            IrOp::If {
                then_body,
                else_body,
                ..
            } => has_match(then_body) || has_match(else_body),
            _ => false,
        })
    }
    assert!(has_match(&function(&ir, "parse").body));
}

#[test]
fn inline_trap_handler_diverges_with_return() {
    let ir = lower_src(
        "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n    RETURN 99\n  END TRAP\n\
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

// ---- union wrapping --------------------------------------------------

#[test]
fn returned_variant_wraps_into_union() {
    let ir = lower_src(
        "TYPE Dog\n  legs AS Integer\nEND TYPE\n\
             TYPE Cat\n  lives AS Integer\nEND TYPE\n\
             UNION Animal\n  Dog\n  Cat\nEND UNION\n\
             FUNC makeDog() AS Animal\n  RETURN Dog[4]\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "makeDog").body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::UnionWrap { .. }),
            ..
        }
    )));
}

#[test]
fn union_typed_binding_wraps_variant() {
    let ir = lower_src(
        "TYPE Dog\n  legs AS Integer\nEND TYPE\n\
             UNION Animal\n  Dog\nEND UNION\n\
             FUNC run() AS Integer\n  LET a AS Animal = Dog[4]\n  RETURN 0\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Bind {
            value: Some(IrValue::UnionWrap { .. }),
            ..
        }
    )));
}

fn describe_all(values: &[IrValue]) -> String {
    values.iter().map(describe).collect::<Vec<_>>().join("; ")
}

/// bug-990: the argument `show` receives in each `Eval` statement of `run`,
/// in source order.
fn show_arguments(ir: &IrProject) -> Vec<IrValue> {
    function(ir, "run")
        .body
        .iter()
        .filter_map(|op| match op {
            IrOp::Eval {
                value: IrValue::Call { target, args, .. },
                ..
            } if target == "show" => args.first().cloned(),
            _ => None,
        })
        .collect()
}

#[test]
fn variant_of_every_expression_kind_wraps_at_a_union_argument() {
    // bug-990: a call result, a field read, a `collections::get` result and a
    // `WITH` result typed as the variant `Square` are each wrapped into the
    // `Shape` the parameter expects, around the unchanged inner value.
    let ir = lower_src(
        "IMPORT collections\n\
             TYPE Circle\n  r AS Float\nEND TYPE\n\
             TYPE Square\n  side AS Float\nEND TYPE\n\
             UNION Shape\n  Circle\n  Square\nEND UNION\n\
             TYPE Hold\n  sq AS Square\nEND TYPE\n\
             FUNC show(s AS Shape) AS Nothing\nEND FUNC\n\
             FUNC mk AS Square\n  RETURN Square[2.0]\nEND FUNC\n\
             FUNC run() AS Integer\n\
               LET h AS Hold = Hold[Square[2.0]]\n\
               LET sqs AS List OF Square = [Square[2.0]]\n\
               LET w AS Square = Square[1.0]\n\
               show(mk())\n\
               show(h.sq)\n\
               show(collections::get(sqs, 0))\n\
               show(WITH w { side := 2.0 })\n\
               show(w)\n\
               RETURN 0\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let args = show_arguments(&ir);
    assert_eq!(args.len(), 5, "{}", describe_all(&args));
    let inner_kinds: [fn(&IrValue) -> bool; 5] = [
        |value: &IrValue| matches!(value, IrValue::Call { target, .. } if target == "mk"),
        |value: &IrValue| matches!(value, IrValue::MemberAccess { .. }),
        |value: &IrValue| matches!(value, IrValue::Call { .. }),
        |value: &IrValue| matches!(value, IrValue::WithUpdate { .. }),
        |value: &IrValue| matches!(value, IrValue::Local(name) if name == "w"),
    ];
    for (arg, inner_is_expected_kind) in args.iter().zip(inner_kinds) {
        let IrValue::UnionWrap {
            union_type,
            member_type,
            value,
        } = arg
        else {
            panic!("argument is not union-wrapped: {}", describe(arg));
        };
        assert!(union_type.is_named("Shape"), "{}", describe(arg));
        assert!(member_type.is_named("Square"), "{}", describe(arg));
        // Exactly one wrap: the wrapped value is the producer itself.
        assert!(inner_is_expected_kind(value), "{}", describe(arg));
    }
}

#[test]
fn union_valued_argument_is_not_wrapped_again() {
    // bug-990 non-goal: a value that is already the union gets no wrap, and
    // a constructor is wrapped exactly once.
    let ir = lower_src(
        "TYPE Circle\n  r AS Float\nEND TYPE\n\
             TYPE Square\n  side AS Float\nEND TYPE\n\
             UNION Shape\n  Circle\n  Square\nEND UNION\n\
             FUNC show(s AS Shape) AS Nothing\nEND FUNC\n\
             FUNC mkShape AS Shape\n  RETURN Square[2.0]\nEND FUNC\n\
             FUNC run() AS Integer\n\
               LET z AS Shape = Circle[1.0]\n\
               show(z)\n\
               show(mkShape())\n\
               show(Square[2.0])\n\
               RETURN 0\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let args = show_arguments(&ir);
    assert_eq!(args.len(), 3, "{}", describe_all(&args));
    assert!(
        matches!(&args[0], IrValue::Local(name) if name == "z"),
        "{}",
        describe(&args[0])
    );
    assert!(
        matches!(&args[1], IrValue::Call { target, .. } if target == "mkShape"),
        "{}",
        describe(&args[1])
    );
    let IrValue::UnionWrap { value, .. } = &args[2] else {
        panic!(
            "constructor argument is not union-wrapped: {}",
            describe(&args[2])
        );
    };
    assert!(
        matches!(**value, IrValue::Constructor { .. }),
        "{}",
        describe(&args[2])
    );
}

// ---- native LINK / RESOURCE / aliases --------------------------------

#[test]
fn link_functions_resources_and_aliases() {
    let src = "EXPORT RESOURCE Db CLOSE BY demoLink::close\n\
             LINK \"sqlite3\" AS demoLink\n\
               FUNC open(path AS String) AS RES Db\n\
                 SYMBOL \"sqlite3_open\"\n\
                 ABI (path CString, produced OUT CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
               FUNC close(db AS Db) AS Nothing\n\
                 SYMBOL \"sqlite3_close\"\n\
                 ABI (db CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
             END LINK\n\
             EXPORT FUNC close AS demoLink::close\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("LINK program should lower");
    let open = ir
        .link_functions
        .iter()
        .find(|f| f.name == "open")
        .expect("open link function");
    assert_eq!(open.library, "sqlite3");
    assert_eq!(open.symbol, "sqlite3_open");
    assert!(open.success_on.is_some());
    assert!(!open.abi_slots.is_empty());

    let db = ir
        .native_resources
        .iter()
        .find(|r| r.name == "Db")
        .expect("Db resource");
    assert_eq!(db.visibility, "export");
    assert!(db.close_may_fail);

    assert!(ir
        .link_aliases
        .iter()
        .any(|(name, target)| name == "close" && target == "demoLink.close"));
}

#[test]
fn link_const_pin_and_result_expression() {
    let src = "RESOURCE Handle CLOSE BY lib::shut\n\
             LINK \"c\" AS lib\n\
               FUNC make() AS RES Handle\n\
                 SYMBOL \"make\"\n\
                 ABI (flags CInt32, produced OUT CPtr) AS status CInt32\n\
                 CONST flags = 0\n\
                 SUCCESS_ON status = 0\n\
                 RETURN NOT (status < 0)\n\
               END FUNC\n\
               FUNC shut(h AS Handle) AS Nothing\n\
                 SYMBOL \"shut\"\n\
                 ABI (h CPtr) AS status CInt32\n\
               END FUNC\n\
             END LINK\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("link CONST/RETURN program should lower");
    let make = ir
        .link_functions
        .iter()
        .find(|f| f.name == "make")
        .expect("make link function");
    assert!(!make.consts.is_empty());
    assert!(make.result.is_some());
    let handle = ir.native_resources.iter().find(|r| r.name == "Handle");
    assert!(handle.is_some_and(|r| !r.close_may_fail));
}

// A `CONST slot = SIZEOF <CStruct>` pin folds to the struct's C layout size
// (eval_link_const_opt SIZEOF arm).
#[test]
fn link_const_sizeof_folds_to_layout_size() {
    let src = "RESOURCE H CLOSE BY lib::shut\n\
             TYPE Rec\n  a AS Integer\n  b AS Integer\nEND TYPE\n\
             LINK \"c\" AS lib\n\
               CSTRUCT Pair AS Rec\n    a CInt32\n    b CInt32\n  END CSTRUCT\n\
               FUNC make() AS RES H\n\
                 SYMBOL \"make\"\n\
                 ABI (sz CInt32, produced OUT CPtr) AS status CInt32\n\
                 CONST sz = SIZEOF Pair\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
               FUNC shut(h AS H) AS Nothing\n\
                 SYMBOL \"shut\"\n\
                 ABI (h CPtr) AS status CInt32\n\
               END FUNC\n\
             END LINK\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("SIZEOF CONST program should lower");
    let make = ir
        .link_functions
        .iter()
        .find(|f| f.name == "make")
        .expect("make link function");
    let sz = make
        .consts
        .iter()
        .find(|(s, _)| s == "sz")
        .expect("sz const");
    assert_eq!(sz.1, 8, "SIZEOF of two CInt32 fields is 8 bytes");
}

// `BIND IN` field values lower to a param reference, an integer literal, a
// boolean (0/1), or a negated literal; an unrecognized form lowers to
// neither (checkers reject it later).
#[test]
fn link_bind_in_field_value_forms() {
    let src = "RESOURCE H CLOSE BY lib::shut\n\
             TYPE Rec\n  a AS Integer\n  b AS Integer\n  c AS Integer\n  d AS Integer\n  e AS Integer\nEND TYPE\n\
             LINK \"c\" AS lib\n\
               CSTRUCT Cfg AS Rec\n    a CInt64\n    b CInt64\n    c CInt64\n    d CInt64\n    e CInt64\n  END CSTRUCT\n\
               FUNC make(seed AS Integer) AS RES H\n\
                 SYMBOL \"make\"\n\
                 ABI (cfg IN Cfg, produced OUT CPtr) AS status CInt32\n\
                 BIND IN cfg\n\
                   a = seed\n\
                   b = 5\n\
                   c = TRUE\n\
                   d = -3\n\
                   e = -seed\n\
                 END BIND\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
               FUNC shut(h AS H) AS Nothing\n\
                 SYMBOL \"shut\"\n\
                 ABI (h CPtr) AS status CInt32\n\
               END FUNC\n\
             END LINK\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("BIND IN program should lower");
    let make = ir
        .link_functions
        .iter()
        .find(|f| f.name == "make")
        .expect("make link function");
    let bind = make
        .bind_in
        .iter()
        .find(|b| b.slot == "cfg")
        .expect("cfg bind");
    let field = |name: &str| bind.fields.iter().find(|f| f.name == name).unwrap();
    assert_eq!(field("a").param.as_deref(), Some("seed"));
    assert_eq!(field("b").literal, Some(5));
    assert_eq!(field("c").literal, Some(1));
    assert_eq!(field("d").literal, Some(-3));
    // `-seed` negates a non-literal → neither a param nor a literal (the
    // unrecognized form the checkers reject later).
    let e = field("e");
    assert_eq!(e.param, None);
    assert_eq!(e.literal, None);
}

// `RETURN <expr>` (the result clause) binary lowering covers the `-`
// arithmetic arm and the `_ => Int(0)` fallback for an operator with no
// LINK-expr encoding.
#[test]
fn link_expr_binary_minus_and_unknown_fallback() {
    let src = "LINK \"c\" AS lib\n\
               FUNC make() AS Integer\n\
                 SYMBOL \"make\"\n\
                 ABI () AS status CInt32\n\
                 RETURN status - 1\n\
               END FUNC\n\
               FUNC probe() AS Integer\n\
                 SYMBOL \"probe\"\n\
                 ABI () AS status CInt32\n\
                 RETURN status / 2\n\
               END FUNC\n\
             END LINK\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("RETURN-expression program should lower");
    let make = ir
        .link_functions
        .iter()
        .find(|f| f.name == "make")
        .expect("make link function");
    assert!(
        matches!(make.result, Some(IrLinkExpr::Sub(_, _))),
        "status - 1 lowers to a Sub expr"
    );
    let probe = ir
        .link_functions
        .iter()
        .find(|f| f.name == "probe")
        .expect("probe link function");
    assert!(
        matches!(probe.result, Some(IrLinkExpr::Int(0))),
        "an unencodable operator lowers to Int(0)"
    );
}

// An `EXPORT FUNC alias AS lib::close` re-export maps the resource's close op
// to the bare exported alias name (native_resources export-alias arm).
#[test]
fn native_resource_close_uses_exported_alias() {
    let src = "EXPORT RESOURCE Db CLOSE BY lib::close\n\
             LINK \"c\" AS lib\n\
               FUNC close(db AS Db) AS Nothing\n\
                 SYMBOL \"close\"\n\
                 ABI (db CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
             END LINK\n\
             EXPORT FUNC shutdown AS lib::close\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("re-export close program should lower");
    let db = ir
        .native_resources
        .iter()
        .find(|r| r.name == "Db")
        .expect("Db resource");
    assert_eq!(db.close_function, "shutdown");
}

// ---- DOC blocks ------------------------------------------------------

#[test]
fn doc_blocks_collected_for_exports() {
    let src = "DOC\n  PACKAGE\n  DESC A package.\nEND DOC\n\
             DOC\n  FUNC add(Integer, Integer)\n  GROUP Math\n  DESC Adds.\n\
               ARG a First.\n  ARG b Second.\n  RET The sum.\nEND DOC\n\
             EXPORT FUNC add(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\n\
             DOC\n  TYPE Point\n  DESC A point.\n  PROP x X.\nEND DOC\n\
             EXPORT TYPE Point\n  x AS Integer\nEND TYPE\n\
             DOC\n  ENUM Color\n  DESC Colors.\nEND DOC\n\
             EXPORT ENUM Color\n  Red\nEND ENUM\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("DOC program should lower");
    assert!(ir.docs.package.is_some());
    let add = ir
        .docs
        .decls
        .iter()
        .find(|d| d.name == "add")
        .expect("add doc");
    assert_eq!(add.group, "Math");
    assert_eq!(add.args.len(), 2);
    assert!(ir
        .docs
        .decls
        .iter()
        .any(|d| d.name == "Point" && d.kind == IrDocKind::Type));
    assert!(ir
        .docs
        .decls
        .iter()
        .any(|d| d.name == "Color" && d.kind == IrDocKind::Enum));
}

#[test]
fn doc_block_for_union_and_nothing_func() {
    let src = "DOC\n  UNION Shape\n  DESC A shape.\nEND DOC\n\
             TYPE Dot\n  n AS Integer\nEND TYPE\n\
             EXPORT UNION Shape\n  Dot\nEND UNION\n\
             DOC INTERNAL\n  FUNC logIt\n  DESC Logs.\n  DEPRECATED Use logger.\nEND DOC\n\
             EXPORT FUNC logIt(value AS Integer) AS Nothing\n  LET x AS Integer = value\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n";
    if let Some(ir) = try_lower_src(src) {
        assert!(ir
            .docs
            .decls
            .iter()
            .any(|d| d.name == "Shape" && d.kind == IrDocKind::Union));
        let log = ir.docs.decls.iter().find(|d| d.name == "logIt");
        assert!(
            log.is_some_and(|d| d.kind == IrDocKind::Func && d.internal && d.deprecated.is_some())
        );
    }
}

#[test]
fn doc_for_nonexported_function_is_dropped() {
    let src = "DOC\n  FUNC helper()\n  DESC Private.\nEND DOC\n\
             FUNC helper() AS Integer\n  RETURN 1\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n";
    if let Some(ir) = try_lower_src(src) {
        assert!(!ir.docs.decls.iter().any(|d| d.name == "helper"));
    }
}

#[test]
fn doc_resource_for_export_is_collected() {
    // A DOC RESOURCE block over an EXPORT RESOURCE is persisted as an
    // IrDocKind::Resource decl (the DocHeaderKind::Resource arm).
    let src = "DOC\n  RESOURCE Db\n  DESC A database handle.\nEND DOC\n\
             EXPORT RESOURCE Db CLOSE BY lib::close\n\
             LINK \"c\" AS lib\n\
               FUNC close(db AS Db) AS Nothing\n\
                 SYMBOL \"close\"\n\
                 ABI (db CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
             END LINK\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("DOC RESOURCE program should lower");
    assert!(ir
        .docs
        .decls
        .iter()
        .any(|d| d.name == "Db" && d.kind == IrDocKind::Resource));
}

#[test]
fn doc_resource_for_nonexport_is_dropped() {
    // A DOC RESOURCE block over a non-exported RESOURCE hits the visibility
    // guard and is not persisted.
    let src = "DOC\n  RESOURCE Cache\n  DESC A cache.\nEND DOC\n\
             PRIVATE RESOURCE Cache CLOSE BY lib::freeCache\n\
             LINK \"c\" AS lib\n\
               FUNC freeCache(c AS Cache) AS Nothing\n\
                 SYMBOL \"freeCache\"\n\
                 ABI (c CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
             END LINK\n\
             FUNC main AS Nothing\nEND FUNC\n";
    if let Some(ir) = try_lower_src(src) {
        assert!(!ir
            .docs
            .decls
            .iter()
            .any(|d| d.name == "Cache" && d.kind == IrDocKind::Resource));
    }
}

// ---- qualified builtin package call ----------------------------------

#[test]
fn qualified_strings_call_lowers_to_internal_target() {
    let ir = lower_src(
        "IMPORT strings\n\
             FUNC up(s AS String) AS String\n  RETURN strings::upper(s)\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "up").body.iter().any(|op| matches!(
        op,
        IrOp::Return {
            value: Some(IrValue::Call { .. }),
            ..
        }
    )));
}

// ---- RES with STATE: param STATE, StateAssign, `.state` access -------

#[test]
fn resource_state_param_assign_and_access() {
    let ir = lower_src(
        "IMPORT fs\n\
             IMPORT io\n\
             TYPE FileState\n  pos AS Integer\n  len AS Integer\nEND TYPE\n\
             FUNC advance(f AS fs::File STATE FileState, by AS Integer) AS Nothing\n\
               f.state = WITH f.state { pos := f.state.pos + by }\n\
             END FUNC\n\
             FUNC main AS Integer\n\
               RES f AS fs::File STATE FileState = fs::createTempFile()\n\
               io::print(toString(f.state.pos))\n\
               advance(f, 5)\n\
               fs::close(f)\n\
               RETURN 0\n\
             END FUNC\n",
    );
    // The `advance` function has a resource param carrying STATE in its type string.
    let advance = function(&ir, "advance");
    assert!(advance.params[0].type_.name().contains("STATE FileState"));
    // Its body assigns the resource STATE via StateAssign.
    assert!(advance
        .body
        .iter()
        .any(|op| matches!(op, IrOp::StateAssign { .. })));
    // `main` reads `f.state.pos` -> a MemberAccess chain over the RES state.
    fn has_member_access(ops: &[IrOp]) -> bool {
        fn value_has(v: &IrValue) -> bool {
            match v {
                IrValue::MemberAccess { .. } => true,
                IrValue::Call { args, .. } => args.iter().any(value_has),
                _ => false,
            }
        }
        ops.iter().any(|op| match op {
            IrOp::Eval { value, .. }
            | IrOp::Bind {
                value: Some(value), ..
            } => value_has(value),
            _ => false,
        })
    }
    assert!(has_member_access(&function(&ir, "main").body));
}

// ---- builtin package calls (expression_type resolution branches) -----

#[test]
fn many_builtin_package_calls_resolve_return_types() {
    // Each qualified builtin routes through its package's expression_type
    // resolver and its `implementation_name`/internalization path.
    let ir = lower_src(
        "IMPORT math\n\
             IMPORT bits\n\
             IMPORT io\n\
             IMPORT datetime\n\
             FUNC compute() AS Integer\n\
               LET a AS Float = math::sqrt(2.0)\n\
               LET b AS Integer = bits::sl(1, 3)\n\
               LET t AS datetime::Instant = datetime::now()\n\
               RETURN b\n\
             END FUNC\n\
             FUNC main AS Nothing\n  io::print(\"x\")\nEND FUNC\n",
    );
    let f = function(&ir, "compute");
    // math::sqrt resolves to a Float-returning call.
    let sqrt_ty = f.body.iter().find_map(|op| match op {
        IrOp::Bind {
            name,
            value: Some(IrValue::Call { type_, .. }),
            ..
        } if name == "a" => Some(type_.name().into_owned()),
        _ => None,
    });
    assert_eq!(sqrt_ty.as_deref(), Some("Float"));
}

#[test]
fn strings_and_collections_calls_resolve() {
    let ir = lower_src(
        "IMPORT strings\n\
             IMPORT collections\n\
             FUNC run() AS Integer\n\
               LET s AS String = strings::upper(\"hi\")\n\
               LET n AS Integer = strings::byteLen(s)\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               LET total AS Integer = collections::sum(xs)\n\
               RETURN n + total\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    let f = function(&ir, "run");
    assert!(f.body.iter().any(|op| matches!(
        op,
        IrOp::Bind {
            name,
            value: Some(IrValue::Call { type_, .. }),
            ..
        } if name == "n" && type_.name() == "Integer"
    )));
}

#[test]
fn collections_filter_with_named_predicate() {
    // `collections::filter(list, predicate)` with a built-in predicate name
    // (`isEven`) takes the filter-predicate-typing + FunctionRef lowering
    // branch (the predicate is resolved to `FUNC(Integer) AS Boolean`).
    let ir = lower_src(
        "IMPORT collections\n\
             FUNC run() AS List OF Integer\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               RETURN collections::filter(xs, isEven)\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    fn has_fn_ref(v: &IrValue) -> bool {
        match v {
            IrValue::FunctionRef { .. } => true,
            IrValue::Call { args, .. } => args.iter().any(has_fn_ref),
            _ => false,
        }
    }
    assert!(function(&ir, "run").body.iter().any(|op| matches!(
        op,
        IrOp::Return { value: Some(v), .. } if has_fn_ref(v)
    )));
}

#[test]
fn regex_call_padding_and_internal_name() {
    // `regex::` calls pad optional trailing arguments and internalize.
    let ir = lower_src(
        "IMPORT regex\n\
             FUNC run() AS Boolean\n  RETURN regex::match(\"abc\", \"a.*\")\nEND FUNC\n\
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
fn nested_call_arguments_capture_and_lower() {
    // Nested built-in call as an argument exercises the argument-lowering
    // recursion and expected-type propagation.
    let ir = lower_src(
        "IMPORT io\n\
             FUNC main AS Nothing\n  io::print(toString(len(\"abc\")))\nEND FUNC\n",
    );
    fn has_nested_call(ops: &[IrOp]) -> bool {
        fn value_has(v: &IrValue) -> bool {
            match v {
                IrValue::Call { args, .. } => {
                    args.iter().any(|a| matches!(a, IrValue::Call { .. }))
                        || args.iter().any(value_has)
                }
                _ => false,
            }
        }
        ops.iter()
            .any(|op| matches!(op, IrOp::Eval { value, .. } if value_has(value)))
    }
    assert!(has_nested_call(main_body(&ir)));
}

// ---- lambda body captures across many expression shapes --------------

#[test]
fn lambda_captures_across_expression_shapes() {
    // The lambda body references captures inside a binary op, a member
    // access, a list literal, a map literal, a constructor, and a nested
    // call — exercising each arm of `collect_captured_locals`.
    let ir = lower_src(
        "IMPORT collections\n\
             TYPE Point\n  x AS Integer\nEND TYPE\n\
             FUNC run() AS List OF Integer\n\
               LET base AS Integer = 1\n\
               LET p AS Point = Point[2]\n\
               LET xs AS List OF Integer = [10]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + base + p.x + len([base]))\n\
             END FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    // Both `base` and `p` are captured by the lambda.
    let lambda = ir
        .functions
        .iter()
        .find(|f| f.name.starts_with("$lambda"))
        .expect("a lambda");
    let capture_binds = lambda
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
    assert!(capture_binds >= 2, "captured at least base and p");
}

// ---- native LINK expression forms (lower_link_expr / eval_link_const) --

#[test]
fn link_success_on_with_comparison_and_boolean_ops() {
    // Exercises eval_link_const (number, boolean, NOTHING, unary +/-) and
    // lower_link_expr (Var identifier, NOTHING, comparison, AND/OR/NOT,
    // non-logic binary fallthrough, unary minus on a Var).
    let src = "RESOURCE H CLOSE BY lib::done\n\
             LINK \"c\" AS lib\n\
               FUNC make() AS RES H\n\
                 SYMBOL \"make\"\n\
                 ABI (a CInt32, b CInt32, c CInt32, produced OUT CPtr) AS status CInt32\n\
                 CONST a = -1\n\
                 CONST b = TRUE\n\
                 CONST c = NOTHING\n\
                 SUCCESS_ON status >= 0 AND status <> 5\n\
                 RETURN status OR NOT (status < 0)\n\
               END FUNC\n\
               FUNC probe() AS RES H\n\
                 SYMBOL \"probe\"\n\
                 ABI (produced OUT CPtr) AS status CInt32\n\
                 SUCCESS_ON status\n\
                 RETURN -status\n\
               END FUNC\n\
               FUNC probe2() AS RES H\n\
                 SYMBOL \"probe2\"\n\
                 ABI (produced OUT CPtr) AS status CInt32\n\
                 SUCCESS_ON status <> NOTHING\n\
                 RETURN status + 1\n\
               END FUNC\n\
               FUNC done(h AS H) AS Nothing\n\
                 SYMBOL \"done\"\n\
                 ABI (h CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
             END LINK\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("link SUCCESS_ON/RETURN program should lower");
    let make = ir
        .link_functions
        .iter()
        .find(|f| f.name == "make")
        .expect("make");
    assert!(make.success_on.is_some());
    assert!(make.result.is_some());
    // CONST pins were evaluated: a=-1, b=TRUE(1), c=NOTHING(0).
    assert_eq!(make.consts.len(), 3);
    // probe2's SUCCESS_ON/RETURN lowered (NOTHING comparison + non-logic
    // binary).
    assert!(ir
        .link_functions
        .iter()
        .any(|f| f.name == "probe2" && f.result.is_some()));
}

/// bug-34: a C flag/mask constant is unsigned, so a pin with bit 63 set
/// exceeds `i64::MAX`. The signed parse failed and `unwrap_or(0)` handed the
/// native call a NULL where the mask belonged, with no diagnostic.
#[test]
fn link_const_keeps_the_exact_bits_of_a_64_bit_flag() {
    let src = "RESOURCE H CLOSE BY lib::done\n\
             LINK \"c\" AS lib\n\
               FUNC make() AS RES H\n\
                 SYMBOL \"make\"\n\
                 ABI (a CInt64, b CInt64, c CInt64, d CInt64, produced OUT CPtr) AS status CInt32\n\
                 CONST a = 0xFFFFFFFFFFFFFFFF\n\
                 CONST b = 9223372036854775808\n\
                 CONST c = -9223372036854775808\n\
                 CONST d = 0x7FFFFFFFFFFFFFFF\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
               FUNC done(h AS H) AS Nothing\n\
                 SYMBOL \"done\"\n\
                 ABI (h CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
             END LINK\n\
             FUNC main AS Nothing\nEND FUNC\n";
    let ir = try_lower_src(src).expect("64-bit CONST pins should lower");
    let make = ir
        .link_functions
        .iter()
        .find(|f| f.name == "make")
        .expect("make");
    let pin = |slot: &str| {
        make.consts
            .iter()
            .find(|(name, _)| name == slot)
            .unwrap_or_else(|| panic!("pin {slot}"))
            .1
    };
    // The ABI cares about the bits, not the sign.
    assert_eq!(pin("a") as u64, 0xFFFF_FFFF_FFFF_FFFF);
    assert_eq!(pin("b") as u64, 1u64 << 63);
    assert_eq!(pin("c"), i64::MIN);
    assert_eq!(pin("d"), i64::MAX);
}
