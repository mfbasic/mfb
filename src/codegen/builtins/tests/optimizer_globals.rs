//! The three Level-2 global rows, as things you can see in the NIR module.
//!
//! `optimizer/opt1/globals.rs` was at 65.97%. It runs only at `-O2` and above
//! and only over a module that HAS a private global worth simplifying, and no
//! corpus fixture has one — so the census, the read-only inference, the
//! constification and the whole `substitute_op` walk over the module were
//! unexecuted.
//!
//! The pass runs on the NIR module, so this drives it there rather than through
//! a code plan: a module-level `LET` never reaches a code plan as a data object
//! at all (measured — the plan's `data_objects` are string constants and the
//! arena), so a test reading the plan would be looking in the wrong place and
//! finding nothing, which reads exactly like a pass that did nothing.
//!
//! Each row is asserted by what it does to the module:
//!
//!   - **Dead global elimination** — a private global nothing names is removed.
//!   - **Read-only inference** — a private global nothing WRITES has its
//!     `mutable` flag cleared, so storage planning may place it in the
//!     read-only partition.
//!   - **Constification** — every read of such a global becomes its initializer,
//!     so `NirValue::Global` naming it stops appearing anywhere in the module.
//!
//! And the two guards that make all three safe, because a pass being right
//! about what it may change is only half the property: an EXPORTED global may be
//! read or written by an importer this module cannot see, and a WRITTEN one has
//! more than one value. Neither may be touched, at any level.

use crate::optimizer::{with_opt_level, OptLevel};
use crate::target::shared::nir::{NirModule, NirOp, NirValue};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};

/// `LIMIT` is private and never written; every kind of statement reads it, so
/// the substitution walk has to reach each arm of `substitute_op` — a bind, an
/// assignment, a store to another global, a return, a condition, a `MATCH`
/// scrutinee and guard, a loop condition and a `FAIL`.
///
/// All three are `PRIVATE`: `escapes()` is `visibility != "private"`, and a
/// module-level binding without the keyword lowers as `public`, which every row
/// refuses outright. Measured — the first version of this suite declared them
/// bare, the pass touched nothing, and all three assertions failed together.
///
/// `TABLE` is a `MUT` nobody writes, which is what the read-only inference is
/// FOR
/// (a `LET` is already immutable at declaration, so it has nothing to infer) —
/// and its initializer is a list literal rather than a `Const`, which is what
/// makes the inference OBSERVABLE. Constification substitutes only a literal, so
/// a never-written `Const` global has its reads folded and is then collected by
/// the dead row in the same pass; the cleared `mutable` flag goes with it. A
/// never-written NON-literal keeps its storage, and the flag is the only thing
/// that changed.
///
/// `TALLY` is written, so it is the control: same shape, opposite verdict.
/// `NEVER_NAMED` is named by nothing at all.
const SRC: &str = "\
IMPORT io

PRIVATE LET LIMIT AS Integer = 7
PRIVATE LET NEVER_NAMED AS Integer = 99
PRIVATE MUT TABLE AS List OF Integer = [1, 2, 3]
PRIVATE MUT TALLY AS Integer = 0

FUNC over(n AS Integer) AS Boolean
  RETURN n > LIMIT
END FUNC

FUNC bounded(n AS Integer) AS Integer
  IF n > LIMIT THEN
    FAIL error(77050002, \"over \" & toString(LIMIT))
  END IF
  RETURN n
END FUNC

FUNC describe(n AS Integer) AS String
  MATCH n
    CASE 0
      RETURN \"zero\"
    CASE 1 WHEN n > LIMIT
      RETURN \"big\"
    CASE ELSE
      RETURN \"small\"
  END MATCH
END FUNC

FUNC main() AS Integer
  LET headroom AS Integer = LIMIT + len(TABLE)
  MUT i AS Integer = 0
  WHILE i < LIMIT
    TALLY = TALLY + i
    i = i + 1
  END WHILE
  i = LIMIT
  io::print(toString(headroom) & \" \" & toString(i) & \" \" & toString(TALLY))
  io::print(describe(TALLY) & \" \" & toString(over(TALLY)))
  io::print(toString(bounded(1)))
  RETURN 0

  TRAP(e)
    io::print(\"trapped \" & toString(e.code))
    RETURN 1
  END TRAP
END FUNC
";

/// The module the program lowers to, with the global rows applied at `level`.
///
/// **The synthetic global-initializer function is dropped first, and that is
/// bug-552.** `lower_functions` prepends a private SUB whose body is one
/// `StoreGlobal` per binding, and `census` counts every `StoreGlobal` as a
/// write — so `never_written()` (`writes == 0`) and `untouched()` are false for
/// every global that has an initializer, which in MFBASIC is every global.
/// Measured: over this program the census reports
/// `LIMIT reads=7 writes=1 / NEVER_NAMED reads=0 writes=1 /
/// SETTLED reads=1 writes=1 / TALLY reads=4 writes=2`, all four writes in
/// `__mfb_init_globals_test`, and `simplify` at `-O2` changes nothing at all.
///
/// So this drives the pass with the input it is WRITTEN for rather than the one
/// a program produces. That is a compromise and it is deliberate: the rows'
/// contracts and both their guards are worth pinning now, and the bug report
/// records that nothing reaches them. When the census answers the question its
/// own doc comment asks — "never written AFTER ITS INITIALIZER" — this should
/// lower the real module and [`drop_the_global_initializer`] should go.
fn module_at(level: u8, source: &str) -> NirModule {
    let mut module = nir_for_src(source, CodeTarget::LinuxX86_64, Console)
        .unwrap_or_else(|err| panic!("the program must lower to NIR: {err}"));
    drop_the_global_initializer(&mut module);
    with_opt_level(OptLevel(level), || {
        crate::optimizer::opt1::globals::simplify(&mut module)
    });
    module
}

/// Remove the synthetic `__mfb_init_globals_*` SUB. See [`module_at`].
///
/// The whole function rather than its stores: leaving a store to a global the
/// dead row is about to remove would make the module one the NIR validator
/// refuses ("global store targets unknown global"), which is bug-552's latent
/// hazard and not this suite's subject.
fn drop_the_global_initializer(module: &mut NirModule) {
    let initializer = crate::target::shared::nir::global_initializer_name(&module.project);
    let before = module.functions.len();
    module
        .functions
        .retain(|function| function.name != initializer);
    assert_eq!(
        module.functions.len() + 1,
        before,
        "the program declares globals, so lowering must have prepended \
         `{initializer}` -- if it stopped doing that, this helper is removing \
         nothing and every row below is being asserted against an input it \
         already handles"
    );
}

/// Whether any value anywhere in the module reads the global `name`.
///
/// A whole-module walk rather than a look at the globals list: constification
/// is about the READS, and a pass that cleared a flag without replacing them
/// would leave the list looking right and the program still loading from
/// storage.
fn reads_global(module: &NirModule, name: &str) -> bool {
    fn in_value(value: &NirValue, name: &str) -> bool {
        if matches!(value, NirValue::Global { name: n, .. } if n == name) {
            return true;
        }
        match value {
            NirValue::Call { args, .. }
            | NirValue::CallResult { args, .. }
            | NirValue::Constructor { args, .. }
            | NirValue::RuntimeCall { args, .. } => args.iter().any(|a| in_value(a, name)),
            NirValue::Binary { left, right, .. } => in_value(left, name) || in_value(right, name),
            NirValue::Unary { operand, .. } => in_value(operand, name),
            NirValue::UnionWrap { value, .. }
            | NirValue::UnionExtract { value, .. }
            | NirValue::Checked { value, .. }
            | NirValue::ResultIsOk { value }
            | NirValue::ResultValue { value }
            | NirValue::ResultError { value } => in_value(value, name),
            _ => false,
        }
    }

    fn in_op(op: &NirOp, name: &str) -> bool {
        match op {
            NirOp::Bind { value, .. } | NirOp::StoreGlobal { value, .. } => {
                value.as_ref().is_some_and(|v| in_value(v, name))
            }
            NirOp::Assign { value, .. } | NirOp::StateAssign { value, .. } => in_value(value, name),
            NirOp::Return { value } => value.as_ref().is_some_and(|v| in_value(v, name)),
            NirOp::Eval { value } => in_value(value, name),
            NirOp::ExitProgram { code } => in_value(code, name),
            NirOp::Fail { error } => in_value(error, name),
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                in_value(condition, name)
                    || then_body.iter().any(|o| in_op(o, name))
                    || else_body.iter().any(|o| in_op(o, name))
            }
            NirOp::Match { value, cases } => {
                in_value(value, name)
                    || cases.iter().any(|case| {
                        case.guard.as_ref().is_some_and(|g| in_value(g, name))
                            || case.body.iter().any(|o| in_op(o, name))
                    })
            }
            NirOp::While {
                condition, body, ..
            }
            | NirOp::DoUntil { condition, body } => {
                in_value(condition, name) || body.iter().any(|o| in_op(o, name))
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                in_value(start, name)
                    || in_value(end, name)
                    || in_value(step, name)
                    || body.iter().any(|o| in_op(o, name))
            }
            NirOp::ForEach { iterable, body, .. } => {
                in_value(iterable, name) || body.iter().any(|o| in_op(o, name))
            }
            NirOp::Trap { body, .. } => body.iter().any(|o| in_op(o, name)),
            _ => false,
        }
    }

    module
        .globals
        .iter()
        .any(|global| global.value.as_ref().is_some_and(|v| in_value(v, name)))
        || module
            .functions
            .iter()
            .any(|function| function.body.iter().any(|op| in_op(op, name)))
}

fn global<'a>(
    module: &'a NirModule,
    name: &str,
) -> Option<&'a crate::target::shared::nir::NirGlobal> {
    module.globals.iter().find(|global| global.name == name)
}

/// At `-O0` nothing is simplified: the globals are all there, mutable as
/// declared, and every read is still a load.
///
/// The row that makes the rest mean something. Every assertion below would hold
/// against a pass that ran at every level, or none.
#[test]
fn the_global_rows_do_nothing_below_their_level() {
    let module = module_at(0, SRC);
    for name in ["LIMIT", "NEVER_NAMED", "TALLY"] {
        assert!(
            global(&module, name).is_some(),
            "-O0 must keep {name}: the rows are Level 2"
        );
    }
    assert!(
        reads_global(&module, "LIMIT"),
        "-O0 must still load LIMIT rather than fold it"
    );
}

/// A private global nothing names loses its storage.
#[test]
fn a_global_nothing_names_is_removed() {
    let module = module_at(2, SRC);
    assert!(
        global(&module, "NEVER_NAMED").is_none(),
        "nothing reads or writes NEVER_NAMED, and nothing can observe storage \
         that nothing names"
    );
    assert!(
        global(&module, "TALLY").is_some(),
        "TALLY is written, so its storage is live"
    );
}

/// A private global nothing WRITES becomes read-only, and its reads become the
/// value.
///
/// Both halves, because they are two rows over one proof: clearing the flag
/// without folding the reads leaves every load in place and buys only the
/// storage partition, and folding without clearing it leaves writable storage
/// nothing writes.
#[test]
fn a_never_written_literal_global_is_folded_away_and_then_collected() {
    let module = module_at(2, SRC);
    assert!(
        !reads_global(&module, "LIMIT"),
        "every read of a never-written global with a literal initializer is that \
         literal; one left as a load is a memory access the pass promised to \
         remove, and it blocks the folding rows that run after it"
    );
    // The two rows compose, deliberately: "Dead globals last, so a global whose
    // only reads were just replaced by its own literal is now unmentioned and
    // collectible in this same pass."
    assert!(
        global(&module, "LIMIT").is_none(),
        "with every read folded, nothing names LIMIT any more, so its storage \
         goes too -- folding that left the storage behind would buy the \
         immediate and keep the bytes"
    );
}

/// A never-written global whose initializer is NOT a literal keeps its storage
/// and its reads, and becomes read-only.
///
/// This is where the read-only row is visible on its own. Constification
/// substitutes only a `Const`, so a never-written literal is folded and then
/// collected and its cleared flag goes with it; a never-written LIST is left in
/// place, and the flag is the only thing that changed about it.
#[test]
fn a_never_written_global_the_folder_cannot_touch_is_still_read_only() {
    let module = module_at(2, SRC);
    let table = global(&module, "TABLE").expect("a non-literal initializer is not folded away");
    assert!(
        !table.mutable,
        "nothing writes TABLE, so its initializer is the only value its storage \
         ever holds and storage planning may put it in the read-only partition"
    );
    assert!(
        reads_global(&module, "TABLE"),
        "a list initializer allocates, so duplicating it into every use would \
         change how often that happens -- its reads must stay loads"
    );
}

/// A global that IS written keeps its storage, its flag and its reads.
#[test]
fn a_written_global_is_left_alone() {
    let module = module_at(2, SRC);
    let tally = global(&module, "TALLY").expect("TALLY is still declared");
    assert!(
        tally.mutable,
        "TALLY is stored to, so its storage is not read-only -- placing it in the \
         read-only partition would fault on the first write"
    );
    assert!(
        reads_global(&module, "TALLY"),
        "a written global has more than one value, so no read of it can be \
         replaced by its initializer"
    );
}

/// An EXPORTED global is untouched, however it is used.
///
/// The guard the other two rows rest on: an importer this module cannot see may
/// read it, or write it, so neither the never-written proof nor the
/// nothing-names-it proof holds for one.
#[test]
fn an_exported_global_is_never_simplified() {
    // A module-level binding is PUBLIC unless it says `PRIVATE`, and
    // `escapes()` is `visibility != "private"` -- so dropping the keyword is
    // what makes one escape. (`EXPORT LET` is not the spelling; it does not
    // parse.)
    let exported = SRC
        .replace("PRIVATE LET LIMIT", "LET LIMIT")
        .replace("PRIVATE LET NEVER_NAMED", "LET NEVER_NAMED");
    let module = module_at(2, &exported);

    assert!(
        global(&module, "LIMIT").is_some(),
        "LIMIT is exported: an importer may read it, so this module's \
         nothing-names-it proof does not hold and its storage must stay"
    );
    assert!(
        reads_global(&module, "LIMIT"),
        "an exported global's reads must stay loads -- the value an importer \
         wrote is not the initializer"
    );
    assert!(
        global(&module, "NEVER_NAMED").is_some(),
        "NEVER_NAMED is exported: nothing in THIS module names it, which says \
         nothing about the importers that can"
    );
}
