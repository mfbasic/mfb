# bug-552: the three Level-2 global optimizer rows cannot fire for any global

Last updated: 2026-09-12
Effort: small to make them fire; MEDIUM, because making the dead-global row fire
without a second change produces a module the NIR validator refuses
Severity: LOW (a missed optimization, not a miscompile) — but see "the latent
hazard", which is what makes it worth more than a shrug
Class: Dead optimization / Invariant

Status: **FIXED.** Both halves landed together, as the report said they had to.

## What the rows are

`optimizer/opt1/globals.rs` implements three Level-2 rows over private globals
(`planning/optimizations.md`), all reading one whole-module census:

  - **Dead global elimination** — a private global no function and no other
    global's initializer mentions is removed outright.
  - **Global constification** — a private global that is never written keeps its
    initializer as its only value, so every read is replaced by that literal.
  - **Read-only memory inference** — the same never-written proof, recorded by
    clearing `mutable`, so storage planning may place it in the read-only
    partition.

## They never fire

Both proofs go through `GlobalUse`:

```rust
/// Never written after its initializer — the read-only question.
pub(crate) fn never_written(&self) -> bool {
    self.writes == 0
}

/// Nothing in the module names it.
pub(crate) fn untouched(&self) -> bool {
    self.reads == 0 && self.writes == 0
}
```

and `census` counts one write per `NirOp::StoreGlobal`. **A global's initializer
IS a `StoreGlobal`.** `lower_functions` prepends a synthetic private SUB named
`__mfb_init_globals_<project>` "whose body is one `StoreGlobal` per binding"
(`mfb spec architecture native-ir` §, and `nir::symbols::global_initializer_name`).

So every global with an initializer has `writes >= 1`, and in MFBASIC a
module-level binding always has one. Measured, over a program with four private
globals — one `LET` read seven times, one `LET` read nowhere, one `MUT` read once
and never stored to, one `MUT` stored to twice:

    PROBE-USAGE LIMIT       reads=7 writes=1
    PROBE-USAGE NEVER_NAMED reads=0 writes=1
    PROBE-USAGE SETTLED     reads=1 writes=1
    PROBE-USAGE TALLY       reads=4 writes=2

    PROBE-WRITE LIMIT       in fn __mfb_init_globals_test
    PROBE-WRITE NEVER_NAMED in fn __mfb_init_globals_test
    PROBE-WRITE SETTLED     in fn __mfb_init_globals_test
    PROBE-WRITE TALLY       in fn __mfb_init_globals_test

`never_written()` is false for all four, and `untouched()` is false for all four.
Running `simplify` at `-O2` over that module changes nothing: `NEVER_NAMED` keeps
its storage, `SETTLED` keeps `mutable = true`, and every read of `LIMIT` is still
a load.

The doc comment says the quiet part: "Never written **after its initializer**"
is the question the row wants answered, and `writes == 0` is not that question.

## The latent hazard, which is why this is not just a shrug

Fixing `never_written` alone is a two-line change and would be wrong. The
dead-global row does:

```rust
module.globals.retain(|global| escapes(global) || !after.get(...).untouched());
```

It removes the GLOBAL. It does not remove the `StoreGlobal` in
`__mfb_init_globals_*` that writes it. So the first time that row fires it
produces a module whose initializer stores to a global that no longer exists —
which `target/shared/validate/body.rs` refuses outright:

    NIR global store targets unknown global '<name>'

The deadness has been hiding that. Whoever makes the census answer the right
question has to make the row remove the initializer store in the same change,
and the NIR validator is the thing that will say so.

## How it was found

`planning/tests.md` (the per-file coverage gate task).
`optimizer/opt1/globals.rs` was at 65.97% with the census, both proofs and the
whole `substitute_op` walk unexecuted, and the file's own module doc explains
three rows in detail. Writing the program that should exercise them — four
private globals in every statement shape — produced a module the pass declines
in full, which is what sent the search to the census.

`src/codegen/builtins/tests/optimizer_globals.rs` covers the pass against the
input it is WRITTEN for, by dropping the synthetic initializer function before
running it, and says so in its module doc. That is a deliberate compromise: it
pins the three rows' contracts and both guards, and it does not pretend a real
program reaches them. The day the census answers the right question, that test
should lower to the real module instead and its `drop_the_global_initializer`
helper should go.

## Outcome

Fixed. Two changes, and the report was right that they cannot be separated.

**1. The census now answers the question the row's own name asks.**
`GlobalUse` gained `initializer_writes`, counted separately from `writes`. The
walk knows which function it is in, and a `StoreGlobal` inside
`__mfb_init_globals_*` goes to the new counter. `never_written()` and
`untouched()` are unchanged in form — they now simply mean what they say.

**2. The dead-global row removes the initializer store it orphans.**
`retain` collects the removed names, then strips each `StoreGlobal` naming them
from the initializer SUB. Only the initializer's stores are dropped: a store from
anywhere else would have made the global non-`untouched` and kept it alive, so
there is no live write to lose.

Without (2), the first time the row ever fired it would have emitted a module
`target/shared/validate/body.rs` refuses — "NIR global store targets unknown
global". The deadness had been hiding that, exactly as the report predicted.

### Measured, same program and flags, before and after

```basic
PRIVATE LET LIMIT AS Integer = 7
PRIVATE LET NEVER_NAMED AS Integer = 99
PRIVATE MUT SETTLED AS Integer = 3
PRIVATE MUT TALLY AS Integer = 0
SUB main()
  TALLY = TALLY + LIMIT
  TALLY = TALLY + SETTLED
  io::print("t=" & toString(TALLY))
END SUB
```

`mfb build -v -O2`:

| row | before | after |
|---|---:|---:|
| Dead global elimination | 0 | **3** |
| Global localization / constification | 0 | **2** |
| Read-only memory inference | 0 | **1** |

and the program still prints `t=10` and exits 0 — which is also the proof that
(2) works, because an orphaned initializer store would have failed validation
rather than run.

### A correction to my own first reading, worth recording

The first repro used bare `LET`/`MUT` and the rows still reported 0 after the
census fix. That looked like a second, deeper blocker — every global lowering as
`"visibility": "public"`, which `escapes()` (`visibility != "private"`) refuses.

It is not a bug. MFBASIC has an explicit `PRIVATE` keyword for a module-level
binding, and `optimizer_globals.rs`'s own module doc already records the fact:
"a module-level binding without the keyword lowers as `public`, which every row
refuses outright. Measured — the first version of this suite declared them bare,
the pass touched nothing, and all three assertions failed together." The same
trap, in the same file, twice. The rows are correctly scoped to `PRIVATE`
globals; the bare-binding program was the wrong instrument.

### The test compromise is gone

`src/codegen/builtins/tests/optimizer_globals.rs` used to drop the synthetic
initializer before running the pass, and its doc said so: "When the census
answers the question its own doc comment asks … this should lower the real module
and `drop_the_global_initializer` should go." It does, and it has. The suite now
drives the module a program actually produces, and asserts the initializer is
present rather than removing it.

It also gained `the_initializer_does_not_store_to_a_removed_global`, which pins
the latent hazard structurally — every `StoreGlobal` surviving in the
initializer names a global that is still present — plus a guard that the row
really removed something, so the assertion cannot pass vacuously. That test
could not have been written against the dropped module.

### Instrument

**The artifact gate is blind to this change and says so with a zero**: 1428
tests, 1594 builds, 2003 goldens, **0 diffs** — the byte-identity fixtures build
at the default `-O1`, and these are Level-2 rows, so nothing they cover moves.
The instruments that do cover it are the unit suite, `mfb build -v`'s per-row
fire counts above, and `MFB_OPT=2 scripts/test-accept.sh`.
