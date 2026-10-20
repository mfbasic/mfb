use super::*;
use crate::codegen::engine::builder::TypeModel;
use crate::codegen::memory::data::data_objects::collect_function_ref_names;
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

/// Every function of the module, by name — the `callees` map H5 resolves against.
fn callees(module: &NirModule) -> HashMap<String, &NirFunction> {
    module
        .functions
        .iter()
        .map(|f| (f.name.clone(), f))
        .collect()
}

/// The helpers every caller-side probe calls. `consume` consumes its collection
/// (S11's shape, so `consumable_params` says so); `peek` only reads it.
const HELPERS: &str = "
FUNC consume(xs AS List OF Integer, v AS Integer) AS List OF Integer
  RETURN collections::append(xs, v)
END FUNC

FUNC peek(xs AS List OF Integer) AS Integer
  RETURN len(xs)
END FUNC

FUNC pair(a AS List OF Integer, b AS List OF Integer) AS List OF Integer
  RETURN collections::append(a, len(b))
END FUNC
";

fn probe(body: &str) -> String {
    format!("IMPORT collections\nIMPORT io\nIMPORT fs\n{HELPERS}\n{body}\n")
}

/// How many arguments of `name`'s body may be handed over, with no parameter of
/// `name` owned (the BASE lowering).
fn approved(source: &str, name: &str) -> usize {
    approved_owning(source, name, &[])
}

/// [`approved`] for an owned variant: `owned` names the parameters this lowering
/// owns, as plan-147-D's variant does.
fn approved_owning(source: &str, name: &str, owned: &[&str]) -> usize {
    let module = lower(source);
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");
    let map = callees(&module);
    let owned: ParamSet = owned.iter().map(|n| (*n).to_string()).collect();
    let callbacks = collect_function_ref_names(&module);
    collect_handover_args(function(&module, name), &model, &map, &owned, &callbacks).len()
}

/// How many argument positions of `name`'s body are built by an IN-PLACE
/// self-update of an owned local, rather than by copying it.
fn arg_updates(source: &str, name: &str, owned: &[&str]) -> usize {
    let module = lower(source);
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");
    let map = callees(&module);
    let owned: ParamSet = owned.iter().map(|n| (*n).to_string()).collect();
    let callbacks = collect_function_ref_names(&module);
    collect_handover_args(function(&module, name), &model, &map, &owned, &callbacks)
        .sites
        .values()
        .map(|site| site.arg_update_mask.count_ones() as usize)
        .sum()
}

/// `consumable_params` of `name`, as a sorted list for comparison.
fn consumable_of(source: &str, name: &str) -> Vec<String> {
    let module = lower(source);
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");
    let mut names: Vec<String> = consumable_params(function(&module, name), &model)
        .into_iter()
        .collect();
    names.sort();
    names
}

/// plan-147-C §3.3, the caller side: one row per shape, each naming the
/// plan-147-A §2.3 row it pins.
#[test]
fn collect_handover_args_follows_the_hand_derived_table() {
    // `(what, the body, how many arguments may be handed over)`.
    let rows: Vec<(&str, &str, usize)> = vec![
        (
            "accumulator threaded through a consuming helper",
            "FUNC main() AS Integer
  MUT acc AS List OF Integer = []
  FOR i = 1 TO 3
    acc = consume(acc, i)
  NEXT
  io::print(toString(len(acc)))
  RETURN 0
END FUNC",
            1,
        ),
        (
            "read again after the call (S2)",
            "FUNC main() AS Integer
  MUT x AS List OF Integer = [1, 2]
  LET y AS List OF Integer = consume(x, 1)
  io::print(toString(len(x)) & toString(len(y)))
  RETURN 0
END FUNC",
            0,
        ),
        (
            "an inline TRAP handler reads it (S3)",
            "FUNC main() AS Integer
  MUT x AS List OF Integer = [1, 2]
  x = consume(x, 1) TRAP(e)
    io::print(toString(len(x)))
    RECOVER x
  END TRAP
  io::print(toString(len(x)))
  RETURN 0
END FUNC",
            0,
        ),
        (
            "RECOVER names the old value (S3)",
            "FUNC main() AS Integer
  MUT x AS List OF Integer = [1, 2]
  x = consume(x, 1) TRAP(e)
    RECOVER x
  END TRAP
  io::print(toString(len(x)))
  RETURN 0
END FUNC",
            0,
        ),
        (
            "RECOVER names something else: the old value is never read",
            "FUNC main() AS Integer
  MUT x AS List OF Integer = [1, 2]
  LET other AS List OF Integer = [9]
  x = consume(x, 1) TRAP(e)
    RECOVER other
  END TRAP
  io::print(toString(len(x)))
  RETURN 0
END FUNC",
            1,
        ),
        (
            "a function-level TRAP READS it (S3, trap_live)",
            "FUNC run() AS Integer
  MUT x AS List OF Integer = [1, 2]
  x = consume(x, 1)
  RETURN 0
TRAP(e)
  RETURN len(x)
END TRAP
END FUNC",
            0,
        ),
        (
            "a function-level TRAP that does NOT read it: the positive twin",
            "FUNC run() AS Integer
  MUT x AS List OF Integer = [1, 2]
  x = consume(x, 1)
  RETURN len(x)
TRAP(e)
  RETURN 0
END TRAP
END FUNC",
            1,
        ),
        (
            "the same local reaches two parameters (S6)",
            "FUNC main() AS Integer
  MUT x AS List OF Integer = [1, 2]
  x = pair(x, x)
  io::print(toString(len(x)))
  RETURN 0
END FUNC",
            0,
        ),
        (
            "the helper only reads it (H6)",
            "FUNC main() AS Integer
  MUT x AS List OF Integer = [1, 2]
  LET n AS Integer = peek(x)
  io::print(toString(n))
  RETURN 0
END FUNC",
            0,
        ),
        (
            "a lambda captures it (S9)",
            "FUNC main() AS Integer
  LET x AS List OF Integer = [1, 2]
  LET f AS FUNC(Integer) AS Integer = LAMBDA(k AS Integer) -> len(x) + k
  LET y AS List OF Integer = consume(x, 1)
  io::print(toString(f(0)) & toString(len(y)))
  RETURN 0
END FUNC",
            0,
        ),
        (
            "called through a function value (S13, H5)",
            "FUNC main() AS Integer
  MUT x AS List OF Integer = [1, 2]
  LET f AS FUNC(List OF Integer, Integer) AS List OF Integer = consume
  x = f(x, 1)
  io::print(toString(len(x)))
  RETURN 0
END FUNC",
            0,
        ),
        (
            "a FOR EACH element is not an owned local (H1)",
            "FUNC main() AS Integer
  LET rows AS List OF List OF Integer = [[1], [2]]
  MUT total AS Integer = 0
  FOR EACH row IN rows
    total = total + peek(consume(row, 1))
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC",
            0,
        ),
    ];

    let mut failures = Vec::new();
    for (what, body, want) in rows {
        let source = probe(body);
        let name = if body.contains("FUNC run(") {
            "run"
        } else {
            "main"
        };
        let got = approved(&source, name);
        if got != want {
            failures.push(format!("{what}: {got} argument(s) approved, want {want}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A global argument keeps the value the global had at the call (S7). `Place`
/// names no globals, so this is structural — the row exists so a future `Place`
/// that DID name globals would fail here instead of silently handing one over.
#[test]
fn a_global_argument_is_never_handed_over() {
    let source = probe(
        "MUT g AS List OF Integer = [1, 2]

FUNC main() AS Integer
  g = consume(g, 1)
  io::print(toString(len(g)))
  RETURN 0
END FUNC",
    );
    assert_eq!(approved(&source, "main"), 0);
}

/// A resource-bearing value is excluded by type (S8).
#[test]
fn a_resource_bearing_argument_is_never_handed_over() {
    let source = probe(
        "FUNC countHandles(hs AS List OF RES fs::File) AS List OF RES fs::File
  RETURN collections::append(hs, fs::createTempFile())
END FUNC

FUNC main() AS Integer
  RES a AS fs::File = fs::createTempFile()
  MUT hs AS List OF RES fs::File = [a]
  hs = countHandles(hs)
  io::print(toString(len(hs)))
  RETURN 0
END FUNC",
    );
    assert_eq!(approved(&source, "main"), 0);
}

/// plan-147-E: a fresh temporary argument is handed over, and only to a
/// consumable parameter.
#[test]
fn a_fresh_temporary_argument_is_handed_over() {
    let source = probe(
        "FUNC main() AS Integer
  MUT x AS List OF Integer = [1, 2]
  LET y AS List OF Integer = consume(collections::append(x, 9), 1)
  io::print(toString(len(y)))
  RETURN 0
END FUNC",
    );
    assert_eq!(approved(&source, "main"), 1);
    // `x` is an owned LOCAL here, so the temporary is built in place even in this
    // (base) lowering.
    assert_eq!(arg_updates(&source, "main", &[]), 1);

    // The same temporary to a parameter the callee only READS is refused (H6):
    // handing it over would move a free and nothing else.
    let refused = probe(
        "FUNC main() AS Integer
  MUT x AS List OF Integer = [1, 2]
  LET n AS Integer = peek(collections::append(x, 9))
  io::print(toString(n))
  RETURN 0
END FUNC",
    );
    assert_eq!(approved(&refused, "main"), 0);
}

/// plan-147-E: a parameter is handed ON only inside a lowering that OWNS it.
///
/// Handing the temporary over is not what makes `fill(collections::append(xs, n),
/// n - 1)` flat — that is approved in both lowerings, since a freshly built value
/// has no other reader. What only the variant can do is BUILD that temporary by
/// updating `xs` in place: in the base lowering the caller still owns `xs`, so
/// `append` has to copy it. Measured end to end, that is the difference between
/// 1203 and 12 allocations at N = 600.
#[test]
fn an_owned_parameter_is_handed_on_only_in_a_variant() {
    let source = probe(
        "FUNC fill(xs AS List OF Integer, n AS Integer) AS List OF Integer
  IF n = 0 THEN RETURN xs
  RETURN fill(collections::append(xs, n), n - 1)
END FUNC",
    );
    // The TEMPORARY is handed over either way: a value this statement just built
    // has no other reader, whoever owns `xs`.
    assert_eq!(approved_owning(&source, "fill", &[]), 1);
    assert_eq!(approved_owning(&source, "fill", &["xs"]), 1);
    // What the variant adds is BUILDING that temporary in place. In the base
    // lowering the caller still owns `xs`, so `append` must copy it; in the
    // variant `xs` is this lowering's own block and `append` updates it.
    assert_eq!(arg_updates(&source, "fill", &[]), 0);
    assert_eq!(arg_updates(&source, "fill", &["xs"]), 1);
}

/// plan-147-F: a bare `String` parameter is never handed over — its block may be a
/// static literal, which `arena_free` cannot free — but a `String` FIELD of a
/// record still is, because it travels by value inside the record's own block.
#[test]
fn a_bare_string_parameter_is_never_handed_over() {
    let bare = probe(
        "FUNC widen(s AS String, n AS Integer) AS String
  RETURN s & strings::repeat(\" \", n)
END FUNC

FUNC main() AS Integer
  MUT s AS String = \"abc\"
  s = widen(s, 3)
  io::print(s)
  RETURN 0
END FUNC",
    );
    assert_eq!(consumable_of(&bare, "widen"), Vec::<String>::new());
    assert_eq!(approved(&bare, "main"), 0);

    // The same `String`, as a record FIELD: still handed over, because
    // `record_field_is_pointer` places it by value in the record's own block.
    let field = probe(
        "TYPE Named
  name AS String
  items AS List OF Integer
END TYPE

FUNC addItem(r AS Named, i AS Integer) AS Named
  RETURN WITH r { items := collections::append(r.items, i) }
END FUNC

FUNC main() AS Integer
  MUT r AS Named = Named[name := \"abc\", items := []]
  FOR i = 1 TO 3
    r = addItem(r, i)
  NEXT
  io::print(r.name & toString(len(r.items)))
  RETURN 0
END FUNC",
    );
    assert_eq!(consumable_of(&field, "addItem"), vec!["r".to_string()]);
    assert_eq!(approved(&field, "main"), 1);
}

/// plan-147-F: a record is handed over when every field is, and never when one
/// carries a resource (row S8).
#[test]
fn a_record_is_handed_over_only_when_every_field_is() {
    let ok = probe(
        "TYPE St
  items AS List OF Integer
  seen AS Integer
END TYPE

FUNC addItem(s AS St, i AS Integer) AS St
  RETURN WITH s { items := collections::append(s.items, i) }
END FUNC

FUNC main() AS Integer
  MUT st AS St = St[items := [], seen := 0]
  FOR i = 1 TO 3
    st = addItem(st, i)
  NEXT
  io::print(toString(len(st.items)))
  RETURN 0
END FUNC",
    );
    assert_eq!(consumable_of(&ok, "addItem"), vec!["s".to_string()]);
    assert_eq!(approved(&ok, "main"), 1);

    // A `RES` field makes the whole record ineligible: resources keep pointer
    // semantics, close-once and drop order (row S8).
    let res = probe(
        "TYPE Holder
  handle AS RES fs::File
  items AS List OF Integer
END TYPE

FUNC addItem(h AS Holder, i AS Integer) AS Holder
  RETURN WITH h { items := collections::append(h.items, i) }
END FUNC

FUNC main() AS Integer
  RES f AS fs::File = fs::createTempFile()
  MUT h AS Holder = Holder[handle := f, items := []]
  FOR i = 1 TO 3
    h = addItem(h, i)
  NEXT
  io::print(toString(len(h.items)))
  RETURN 0
END FUNC",
    );
    assert_eq!(consumable_of(&res, "addItem"), Vec::<String>::new());
    assert_eq!(approved(&res, "main"), 0);
}

/// plan-147-C §3.3, the callee side.
#[test]
fn consumable_params_follows_the_hand_derived_table() {
    let source = probe(
        "FUNC setter(xs AS List OF Integer, i AS Integer, v AS Integer) AS List OF Integer
  RETURN collections::set(xs, i, v)
END FUNC

FUNC onePathConsumes(xs AS List OF Integer, n AS Integer) AS List OF Integer
  IF n = 0 THEN RETURN xs
  RETURN []
END FUNC

FUNC onlyReads(xs AS List OF Integer) AS Integer
  RETURN len(xs)
END FUNC

FUNC bindsIt(xs AS List OF Integer, v AS Integer) AS List OF Integer
  MUT acc AS List OF Integer = xs
  acc = collections::append(acc, v)
  RETURN acc
END FUNC",
    );
    assert_eq!(consumable_of(&source, "setter"), vec!["xs".to_string()]);
    assert_eq!(
        consumable_of(&source, "onePathConsumes"),
        vec!["xs".to_string()]
    );
    assert_eq!(consumable_of(&source, "onlyReads"), Vec::<String>::new());
    // plan-147-E: `MUT y = p` is a consuming use too — the bind takes the block.
    assert_eq!(consumable_of(&source, "bindsIt"), vec!["xs".to_string()]);
    // The shared helpers, for the same reason the caller table calls them.
    assert_eq!(consumable_of(&source, "consume"), vec!["xs".to_string()]);
    assert_eq!(consumable_of(&source, "peek"), Vec::<String>::new());
    assert_eq!(consumable_of(&source, "pair"), vec!["a".to_string()]);
}

/// bug-766 C: `RETURN WITH p { … }` over fields stored in `p`'s own block is a
/// consuming use — the owned variant stores into the handed-over block and moves
/// it out — but only when the `RETURN` is `p`'s last read.
#[test]
fn a_returned_with_over_scalar_fields_consumes_its_owner() {
    let source = probe(
        "TYPE R
  n AS Integer
  tag AS String
END TYPE

FUNC bump(p AS R) AS R
  RETURN WITH p { n := p.n + 1 }
END FUNC

FUNC copyN(p AS R, q AS R) AS R
  RETURN WITH p { n := q.n }
END FUNC

FUNC retag(p AS R, t AS String) AS R
  RETURN WITH p { n := p.n + 1, tag := t }
END FUNC

FUNC readLater(p AS R, q AS R) AS Integer
  LET y AS R = WITH p { n := q.n }
  RETURN y.n + p.n
END FUNC

FUNC trapReads(p AS R, q AS R) AS R
  RETURN WITH p { n := q.n }
TRAP(e)
  RETURN p
END TRAP
END FUNC",
    );
    assert_eq!(consumable_of(&source, "bump"), vec!["p".to_string()]);
    // `q` is only read, so only `p` is consumed.
    assert_eq!(consumable_of(&source, "copyN"), vec!["p".to_string()]);
    // bug-766 D: the last inlined `String` field is stored in the owner's block
    // too (in place when its granule is unchanged).
    assert_eq!(consumable_of(&source, "retag"), vec!["p".to_string()]);
    // A `WITH p` that is not the return, with a later read of `p`: nothing is
    // consumed.
    assert_eq!(consumable_of(&source, "readLater"), Vec::<String>::new());
    // A handler reads `p` after the `RETURN WITH` may have failed (row S3).
    assert_eq!(consumable_of(&source, "trapReads"), Vec::<String>::new());
}

/// bug-1019 (H7): a parameter the callee returns as a BORROW is never handed
/// over. `same`'s result is its argument's own pointer and every caller reads
/// it as a borrow of that argument, so the caller must go on owning the block;
/// handed over, it came straight back out of `same$own1` owned by no one.
#[test]
fn a_parameter_returned_as_a_borrow_is_never_handed_over() {
    const BORROWERS: &str = "
FUNC same(xs AS List OF Integer) AS List OF Integer
  RETURN xs
END FUNC

FUNC pick(a AS List OF Integer, b AS List OF Integer, first AS Boolean) AS List OF Integer
  IF first THEN
    RETURN a
  END IF
  RETURN b
END FUNC

FUNC keepFirst(a AS List OF Integer, spare AS List OF Integer) AS List OF Integer
  MUT mine AS List OF Integer = spare
  mine = collections::append(mine, 1)
  io::print(toString(len(mine)))
  RETURN a
END FUNC

FUNC growOrSame(xs AS List OF Integer, n AS Integer) AS List OF Integer
  IF n = 0 THEN
    RETURN xs
  END IF
  RETURN collections::append(xs, n)
END FUNC

FUNC sameRow(row AS List OF Integer) AS List OF Integer
  RETURN row
END FUNC
";
    // `(what, the body of main, how many arguments may be handed over)`.
    let rows: Vec<(&str, &str, usize)> = vec![
        (
            "a fresh temporary through a passthrough",
            "LET y AS List OF Integer = same([1, 2])
  io::print(toString(len(y)))",
            0,
        ),
        (
            "a local at its last use through a passthrough",
            "MUT x AS List OF Integer = [1, 2]
  LET y AS List OF Integer = same(x)
  io::print(toString(len(y)))",
            0,
        ),
        (
            "the temporary built in place from a dead local",
            "MUT x AS List OF Integer = [1, 2]
  LET y AS List OF Integer = same(collections::append(x, 9))
  io::print(toString(len(y)))",
            0,
        ),
        (
            "either of two returned parameters",
            "LET y AS List OF Integer = pick([1], [2], TRUE)
  io::print(toString(len(y)))",
            0,
        ),
        (
            "a parameter the passthrough consumes instead of returning",
            "LET y AS List OF Integer = keepFirst([1], [2])
  io::print(toString(len(y)))",
            1,
        ),
        (
            "a function that also builds its result returns a fresh block",
            "LET y AS List OF Integer = growOrSame([1], 1)
  io::print(toString(len(y)))",
            1,
        ),
        (
            "a passthrough reachable as a FUNC value returns a fresh block",
            "LET rows AS List OF List OF Integer = collections::transform([[1]], sameRow)
  LET y AS List OF Integer = sameRow([1, 2])
  io::print(toString(len(rows) + len(y)))",
            1,
        ),
    ];
    for (what, body, want) in rows {
        let source = probe(&format!(
            "{BORROWERS}\nFUNC main() AS Integer\n  {body}\n  RETURN 0\nEND FUNC"
        ));
        assert_eq!(approved(&source, "main"), want, "{what}");
    }
}
