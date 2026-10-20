use super::*;
use crate::target::shared::nir::NirModule;
use crate::testutil::{nir_for_src, CodeTarget};

fn lower(source: &str) -> NirModule {
    nir_for_src(
        source,
        CodeTarget::MacosAarch64,
        crate::target::NativeBuildMode::Console,
    )
    .unwrap_or_else(|error| panic!("the probe lowers to NIR: {error}"))
}

/// [`lower`] at `-O0`: the probe's hand-written shapes exactly as written. The
/// Opt1 rows are free to rewrite them at the default level — plan-149's place copy
/// forwarding turns `LET b = a; RETURN b.tag` into `RETURN a.tag` — and the table
/// below is about this analysis on each shape, not about what reaches it.
fn lower_unoptimized(source: &str) -> NirModule {
    crate::optimizer::with_opt_level(crate::optimizer::OptLevel(0), || lower(source))
}

fn function<'m>(module: &'m NirModule, name: &str) -> &'m NirFunction {
    module
        .functions
        .iter()
        .find(|f| f.name == name || f.name.rsplit(|c| c == '.' || c == ':').next() == Some(name))
        .unwrap_or_else(|| {
            panic!(
                "no function `{name}` in {:?}",
                module
                    .functions
                    .iter()
                    .map(|f| f.name.as_str())
                    .collect::<Vec<_>>()
            )
        })
}

fn find_ops<'f>(ops: &'f [NirOp], wanted: &dyn Fn(&NirOp) -> bool, out: &mut Vec<&'f NirOp>) {
    for op in ops {
        if wanted(op) {
            out.push(op);
        }
        match op {
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                find_ops(then_body, wanted, out);
                find_ops(else_body, wanted, out);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    find_ops(&case.body, wanted, out);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => find_ops(body, wanted, out),
            _ => {}
        }
    }
}

fn ops<'f>(function: &'f NirFunction, wanted: &dyn Fn(&NirOp) -> bool) -> Vec<&'f NirOp> {
    let mut out = Vec::new();
    find_ops(&function.body, wanted, &mut out);
    out
}

fn binds(name: &'static str) -> impl Fn(&NirOp) -> bool {
    move |op| matches!(op, NirOp::Bind { name: bound, .. } if bound == name)
}

/// A bind whose value is exactly `Local(source)` — how the MATCH desugar binds its view.
fn binds_from_local(source: &'static str) -> impl Fn(&NirOp) -> bool {
    move |op| matches!(op, NirOp::Bind { value: Some(NirValue::Local(from)), .. } if from == source)
}

/// `name = of.field`.
fn assigns_field(
    name: &'static str,
    of: &'static str,
    field: &'static str,
) -> impl Fn(&NirOp) -> bool {
    move |op| match op {
        NirOp::Assign {
            name: target,
            value:
                NirValue::MemberAccess {
                    target: base,
                    member,
                },
        } => {
            target == name
                && member == field
                && matches!(base.as_ref(), NirValue::Local(local) if local == of)
        }
        _ => false,
    }
}

fn is_append_call(value: &NirValue) -> bool {
    match value {
        NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. } => target.contains("append"),
        _ => false,
    }
}

fn appends_to(name: &'static str) -> impl Fn(&NirOp) -> bool {
    move |op| matches!(op, NirOp::Assign { name: target, value } if target == name && is_append_call(value))
}

/// `name = append(name, <place>)`: an append of one particular place, for a
/// function that appends several different values to the same list.
fn appends_place_to(name: &'static str, place: Place) -> impl Fn(&NirOp) -> bool {
    move |op| {
        let NirOp::Assign {
            name: target,
            value,
        } = op
        else {
            return false;
        };
        let NirValue::Call { args, .. } = value else {
            return false;
        };
        if target != name || !is_append_call(value) {
            return false;
        }
        match (&place, args.get(1)) {
            (Place::Local(want), Some(NirValue::Local(got))) => want == got,
            (Place::Field(base, field), Some(NirValue::MemberAccess { target, member })) => {
                member == field && matches!(target.as_ref(), NirValue::Local(b) if b == base)
            }
            _ => false,
        }
    }
}

fn returns_value(op: &NirOp) -> bool {
    matches!(op, NirOp::Return { value: Some(_) })
}

fn local(name: &str) -> Place {
    Place::Local(name.to_string())
}

fn member(name: &str, field: &str) -> Place {
    Place::Field(name.to_string(), field.to_string())
}

/// One function per shape of plan-134-C §Phase 1 (and the MATCH views plan-134-D's
/// speed gate needed), called from `main` so every one is lowered.
const SHAPES: &str = "IMPORT io
IMPORT collections

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

TYPE Link
  rest AS Chain
  v AS Integer
END TYPE

TYPE Stop
  none AS Boolean
END TYPE

UNION Chain
  Link
  Stop
END UNION

FUNC moveAfterBind() AS Integer
  LET a AS Node = Node[kids := [], tag := 1]
  LET b AS Node = a
  RETURN b.tag
END FUNC

FUNC readAgain() AS Integer
  LET a AS Node = Node[kids := [], tag := 1]
  LET b AS Node = a
  RETURN a.tag + b.tag
END FUNC

FUNC nextIteration() AS Integer
  LET a AS Node = Node[kids := [], tag := 1]
  MUT xs AS List OF Node = []
  MUT i AS Integer = 0
  WHILE i < 3
    xs = collections::append(xs, a)
    i = i + 1
  END WHILE
  RETURN len(xs)
END FUNC

FUNC reboundEachIteration() AS Integer
  MUT xs AS List OF Node = []
  MUT i AS Integer = 0
  WHILE i < 3
    LET item AS Node = Node[kids := [], tag := i]
    xs = collections::append(xs, item)
    i = i + 1
  END WHILE
  RETURN len(xs)
END FUNC

FUNC readInHandler() AS Integer
  LET a AS Node = Node[kids := [], tag := 7]
  LET b AS Node = a
  RETURN b.tag + toInt(\"7\")
  TRAP(err)
    RETURN a.tag
  END TRAP
END FUNC

FUNC capturedEarlier() AS Integer
  LET a AS Node = Node[kids := [], tag := 1]
  LET f = LAMBDA(n AS Integer) -> a.tag + n
  LET b AS Node = a
  RETURN f(b.tag)
END FUNC

FUNC forEachLive() AS Integer
  LET xs AS List OF Node = [Node[kids := [], tag := 1]]
  MUT n AS Integer = 0
  FOR EACH e IN xs
    LET copy AS List OF Node = xs
    n = n + len(copy) + e.tag
  NEXT
  RETURN n
END FUNC

FUNC fromParam(p AS Node) AS Integer
  LET q AS Node = p
  RETURN q.tag
END FUNC

FUNC fieldThenOtherField() AS Integer
  LET h AS Node = Node[kids := [], tag := 3]
  LET k AS List OF Node = h.kids
  RETURN h.tag + len(k)
END FUNC

FUNC returnLocal() AS Node
  LET n AS Node = Node[kids := [], tag := 1]
  RETURN n
END FUNC

FUNC isLink(ch AS Chain) AS Boolean
  MATCH ch
    CASE Link(l)
      RETURN TRUE
    CASE ELSE
      RETURN FALSE
  END MATCH
END FUNC

FUNC sumChain(start AS Chain) AS Integer
  MUT cur AS Chain = start
  MUT n AS Integer = 0
  WHILE TRUE
    MATCH cur
      CASE Link(l)
        cur = l.rest
        n = n + l.v
      CASE Stop(s)
        RETURN n
    END MATCH
  END WHILE
  RETURN n
END FUNC

FUNC sumChainOpen(start AS Chain) AS Integer
  MUT cur AS Chain = start
  MUT n AS Integer = 0
  MUT going AS Boolean = TRUE
  WHILE going
    MATCH cur
      CASE Link(l)
        cur = l.rest
        n = n + l.v
      CASE Stop(s)
        going = FALSE
    END MATCH
  END WHILE
  RETURN n
END FUNC

FUNC main AS Integer
  LET r AS Node = returnLocal()
  LET stop AS Chain = Stop[none := TRUE]
  LET chain AS Chain = Link[rest := stop, v := 4]
  io::print(toString(moveAfterBind() + readAgain() + nextIteration() + reboundEachIteration() + readInHandler() + capturedEarlier() + forEachLive() + fromParam(r) + fieldThenOtherField() + sumChain(chain) + sumChainOpen(chain)))
  io::print(toString(isLink(chain)))
  RETURN 0
END FUNC
";

/// The analysis's answer for every shape, against the table derived by hand from
/// the source above. Each row names the store op, the place it reads, and whether
/// that read is the last.
#[test]
fn collect_last_use_moves_follows_the_hand_derived_table() {
    let module = lower_unoptimized(SHAPES);
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");
    let rows: Vec<(&str, &str, Box<dyn Fn(&NirOp) -> bool>, Place, bool)> = vec![
        (
            "never read again",
            "moveAfterBind",
            Box::new(binds("b")),
            local("a"),
            true,
        ),
        (
            "read again after",
            "readAgain",
            Box::new(binds("b")),
            local("a"),
            false,
        ),
        (
            "read on the next iteration",
            "nextIteration",
            Box::new(appends_to("xs")),
            local("a"),
            false,
        ),
        (
            "rebound each iteration, then appended",
            "reboundEachIteration",
            Box::new(appends_to("xs")),
            local("item"),
            true,
        ),
        (
            "read in the TRAP handler",
            "readInHandler",
            Box::new(binds("b")),
            local("a"),
            false,
        ),
        (
            "captured by a lambda",
            "capturedEarlier",
            Box::new(binds("b")),
            local("a"),
            false,
        ),
        (
            "the live FOR EACH iterable",
            "forEachLive",
            Box::new(binds("copy")),
            local("xs"),
            false,
        ),
        (
            "a parameter",
            "fromParam",
            Box::new(binds("q")),
            local("p"),
            false,
        ),
        (
            "a field, another field read after",
            "fieldThenOtherField",
            Box::new(binds("k")),
            member("h", "kids"),
            true,
        ),
        (
            "RETURN of an owned local",
            "returnLocal",
            Box::new(returns_value),
            local("n"),
            true,
        ),
    ];
    for (shape, name, wanted, place, expected) in rows {
        let f = function(&module, name);
        let sites = collect_last_use_moves(f, &model);
        let found = ops(f, wanted.as_ref());
        assert!(
            !found.is_empty(),
            "{shape}: `{name}` has no op of the expected shape"
        );
        for op in found {
            assert_eq!(
                sites.is_last_use(op_key(op), &place),
                expected,
                "{shape} (`{name}`): is {place:?} read for the last time?"
            );
        }
    }
}

/// The MATCH desugar's scrutinee temporary (plan-134-D speed gate, Corrections): a
/// MATCH on a parameter only inspects it (a borrow, never a copy); when every case
/// reassigns or leaves, the temporary owns the scrutinee and a case alias's field can
/// move (`cur = l.rest`); when one case falls back to the loop without reassigning,
/// the source stays live, the temporary borrows, and the alias's field does not move.
#[test]
fn collect_last_use_moves_views_of_a_match_scrutinee() {
    let module = lower(SHAPES);
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");

    let f = function(&module, "isLink");
    let sites = collect_last_use_moves(f, &model);
    let views = ops(f, &binds_from_local("ch"));
    assert_eq!(views.len(), 1, "isLink binds one scrutinee view");
    assert!(
        sites.is_borrow(op_key(views[0])),
        "a MATCH on a parameter borrows"
    );
    assert!(!sites.is_last_use(op_key(views[0]), &local("ch")));

    let f = function(&module, "sumChain");
    let sites = collect_last_use_moves(f, &model);
    let views = ops(f, &binds_from_local("cur"));
    assert_eq!(views.len(), 1, "sumChain binds one scrutinee view");
    assert!(
        sites.is_last_use(op_key(views[0]), &local("cur")),
        "the view owns the chain"
    );
    assert!(!sites.is_borrow(op_key(views[0])));
    let steps = ops(f, &assigns_field("cur", "l", "rest"));
    assert_eq!(steps.len(), 1);
    assert!(
        sites.is_last_use(op_key(steps[0]), &member("l", "rest")),
        "an owning view's alias field moves"
    );

    let f = function(&module, "sumChainOpen");
    let sites = collect_last_use_moves(f, &model);
    let views = ops(f, &binds_from_local("cur"));
    assert_eq!(views.len(), 1, "sumChainOpen binds one scrutinee view");
    assert!(
        sites.is_borrow(op_key(views[0])),
        "a live source makes the view borrow"
    );
    let steps = ops(f, &assigns_field("cur", "l", "rest"));
    assert_eq!(steps.len(), 1);
    assert!(
        !sites.is_last_use(op_key(steps[0]), &member("l", "rest")),
        "a borrowed view's alias never moves"
    );
}

fn hand_built(body: Vec<NirOp>) -> NirFunction {
    NirFunction {
        name: "handBuilt".to_string(),
        visibility: "private".to_string(),
        kind: "function".to_string(),
        isolated: false,
        params: Vec::new(),
        returns: ParameterType::Integer,
        body,
        file: String::new(),
        resource_owners: HashMap::new(),
    }
}

fn zero() -> NirOp {
    NirOp::Return {
        value: Some(NirValue::Const {
            type_: ParameterType::Integer,
            value: "0".to_string(),
        }),
    }
}

/// `r` is bound from a by-ref capture — it is a pointer to a binding in the parent
/// frame, which the parent still owns. The control shows the same shape bound from
/// an ordinary value IS a move, so the exclusion is what the first answer measures.
#[test]
fn collect_last_use_moves_never_moves_a_by_ref_capture() {
    let node = ParameterType::named("Node");
    let bind = |value: NirValue| NirOp::Bind {
        mutable: true,
        name: "r".to_string(),
        type_: node.clone(),
        value: Some(value),
    };
    let copy_out = NirOp::Bind {
        mutable: false,
        name: "s".to_string(),
        type_: node.clone(),
        value: Some(NirValue::Local("r".to_string())),
    };
    let by_ref = hand_built(vec![
        bind(NirValue::Capture {
            index: 0,
            type_: node.clone(),
            by_ref: true,
        }),
        copy_out.clone(),
        zero(),
    ]);
    let sites = collect_last_use_moves(&by_ref, TypeModel::builtin_records());
    assert!(!sites.is_last_use(op_key(&by_ref.body[1]), &local("r")));

    let owned = hand_built(vec![
        bind(NirValue::Const {
            type_: node.clone(),
            value: "0".to_string(),
        }),
        copy_out,
        zero(),
    ]);
    let sites = collect_last_use_moves(&owned, TypeModel::builtin_records());
    assert!(sites.is_last_use(op_key(&owned.body[1]), &local("r")));
}

/// A resource is move-only by its own rules and never copied (plan-134-A
/// non-goals), so the analysis never offers it; the control is an `Integer`.
#[test]
fn collect_last_use_moves_never_moves_a_resource_bearing_value() {
    let model = TypeModel::builtin_records();
    let run = |type_: ParameterType| {
        let f = hand_built(vec![
            NirOp::Bind {
                mutable: false,
                name: "a".to_string(),
                type_: type_.clone(),
                value: Some(NirValue::Const {
                    type_: type_.clone(),
                    value: "0".to_string(),
                }),
            },
            NirOp::Bind {
                mutable: false,
                name: "b".to_string(),
                type_,
                value: Some(NirValue::Local("a".to_string())),
            },
            zero(),
        ]);
        collect_last_use_moves(&f, model).is_last_use(op_key(&f.body[1]), &local("a"))
    };
    // The builtin resource table is keyed by the qualified name.
    let file = ParameterType::declared("fs.File");
    assert!(
        type_contains_resource(model, &file),
        "the probe type must be a resource"
    );
    assert!(!run(file));
    assert!(run(ParameterType::Integer));
}

/// The five `append`s the decoders build their trees with (plan-134-A §2.1), in the
/// real helper bodies: each appends a place read for the last time, so letter D can
/// move instead of deep-copying every element. Three of them append a FIELD of a
/// record whose other fields are read afterwards, and json's element is itself a
/// field read — which is why places are field-sensitive.
const DECODERS: &str = "IMPORT io
IMPORT json
IMPORT regex

FUNC keep(key AS String, value AS json::Json) AS json::Json
  RETURN value
END FUNC

FUNC main AS Integer
  LET doc AS json::Json = json::parse(\"[1,[2]]\")
  LET revived AS json::Json = json::parse(\"[1]\", keep)
  io::print(json::stringify(doc) & json::stringify(revived) & toString(len(regex::findAll(\"ab\", \"a|b\"))))
  RETURN 0
END FUNC
";

#[test]
fn collect_last_use_moves_finds_the_five_decoder_append_sites() {
    let module = lower(DECODERS);
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");
    let expect = |name: &str, wanted: &dyn Fn(&NirOp) -> bool, place: Place, count: usize| {
        let f = function(&module, name);
        let sites = collect_last_use_moves(f, &model);
        let found = ops(f, wanted);
        assert_eq!(found.len(), count, "`{name}`: expected {count} such op(s)");
        for op in found {
            assert!(
                sites.is_last_use(op_key(op), &place),
                "`{name}`: the store of {place:?} must be its last read"
            );
        }
    };
    // A registry helper's `__pkg_name` is spelled `#pkg_name` once lowered.
    // The shipped body at the default level: plan-149's place copy forwarding
    // drops `LET item = parsed.value` (read once, before any write of `parsed`),
    // so the append reads the field itself — and it is still that field's last
    // read, so the element moves with no copy at all (before the forward: one
    // copy into `item`, then a move of `item`).
    expect(
        "#json_parseArrayItems",
        &appends_to("acc"),
        member("parsed", "value"),
        1,
    );
    expect(
        "#json_revive",
        &appends_to("items"),
        local("revivedItem"),
        1,
    );
    expect(
        "#regex_parseAlt",
        &appends_to("opts"),
        member("nextc", "node"),
        1,
    );
    // bug-773 restructured the body: an atom reaches __regex_parseQuantSuffix
    // only when a quantifier follows it, so `parts` receives the quantified node
    // (`q.node`, three branches) or the atom itself (`pr.node` for a group,
    // `atom.node` for `.`/`^`/`$`/a class/an escape, the local `lit` for a plain
    // literal). Every one of the six must still be its value's last read.
    for (place, count) in [
        (member("q", "node"), 3),
        (member("pr", "node"), 1),
        (member("atom", "node"), 1),
        (local("lit"), 1),
    ] {
        expect(
            "#regex_parseConcat",
            &appends_place_to("parts", place.clone()),
            place,
            count,
        );
    }
    assert_eq!(
        ops(
            function(&module, "#regex_parseConcat"),
            &appends_to("parts")
        )
        .len(),
        6,
        "`#regex_parseConcat`: every append to `parts` is one of the six above"
    );
}

/// A linked-chain backtracker in the shape the bug-510 regex matcher had until
/// plan-134-I flattened it: a choice stack and a continuation that are unions of a
/// terminal record and a record holding the rest of the chain.
const CHAIN_BACKTRACKER: &str = "IMPORT io

TYPE BtBottom
  none AS Boolean
END TYPE

TYPE BtFrame
  pos AS Integer
  cont AS BtCont
  nxt AS BtStack
END TYPE

TYPE BtDone
  dummy AS Boolean
END TYPE

TYPE BtSeq
  idx AS Integer
  nxt AS BtCont
END TYPE

UNION BtStack
  BtBottom
  BtFrame
END UNION

UNION BtCont
  BtDone
  BtSeq
END UNION

FUNC walkChains(stack0 AS BtStack, cont0 AS BtCont) AS Integer
  MUT stack AS BtStack = stack0
  MUT cont AS BtCont = cont0
  MUT pos AS Integer = 0
  WHILE TRUE
    MATCH stack
      CASE BtBottom(none)
        RETURN pos
      CASE BtFrame(c)
        stack = c.nxt
        pos = pos + c.pos
        cont = c.cont
    END MATCH
    MATCH cont
      CASE BtDone(doneCont)
        pos = pos + 1
      CASE BtSeq(seqCont)
        pos = pos + seqCont.idx
        cont = seqCont.nxt
    END MATCH
  END WHILE
  RETURN pos
END FUNC

FUNC isDone(node AS BtCont) AS Boolean
  MATCH node
    CASE BtDone(doneNode)
      RETURN TRUE
    CASE ELSE
      RETURN FALSE
  END MATCH
END FUNC

FUNC main AS Integer
  LET chain AS BtStack = BtFrame[3, BtSeq[2, BtDone[TRUE]], BtBottom[TRUE]]
  io::print(toString(walkChains(chain, BtDone[TRUE])) & toString(isDone(BtDone[TRUE])))
  RETURN 0
END FUNC
";

/// The hot stores of a linked-chain backtracker (plan-134-D speed gate): a backtrack
/// pop (`stack = c.nxt`) and a continuation step (`cont = seqCont.nxt`) move the rest
/// of their chain out of an owning MATCH view instead of copying it per step, and a
/// per-step helper's MATCH on its parameter borrows. These were `#regex_run` and
/// `#regex_isSimpleNode` until plan-134-I replaced the matcher's chains with integer
/// tables; `CHAIN_BACKTRACKER` keeps the three shapes under test.
#[test]
fn collect_last_use_moves_covers_the_chain_backtracker_views() {
    let module = lower(CHAIN_BACKTRACKER);
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");

    let run = function(&module, "walkChains");
    let sites = collect_last_use_moves(run, &model);
    let pops = ops(run, &assigns_field("stack", "c", "nxt"));
    assert!(!pops.is_empty(), "walkChains pops its choice stack");
    for op in pops {
        assert!(
            sites.is_last_use(op_key(op), &member("c", "nxt")),
            "a backtrack pop moves the rest of the chain"
        );
    }
    let steps = ops(run, &assigns_field("cont", "seqCont", "nxt"));
    assert!(!steps.is_empty(), "walkChains steps its continuation");
    for op in steps {
        assert!(
            sites.is_last_use(op_key(op), &member("seqCont", "nxt")),
            "a continuation step moves the rest of the continuation"
        );
    }

    let simple = function(&module, "isDone");
    let sites = collect_last_use_moves(simple, &model);
    let views = ops(simple, &binds_from_local("node"));
    assert_eq!(views.len(), 1, "isDone binds one scrutinee view");
    assert!(
        sites.is_borrow(op_key(views[0])),
        "a MATCH on a parameter borrows"
    );
}
