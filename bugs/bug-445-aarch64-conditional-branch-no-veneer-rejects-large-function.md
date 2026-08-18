# bug-445: AArch64 backend rejects a large function instead of relaxing an out-of-range conditional branch (no branch veneer)

Last updated: 2026-08-16
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (valid source rejected by codegen)

Status: Open
Regression Test: tests/rt_aarch64_conditional_branch_relaxation.rs (to be added) + an encoder-level unit test in src/arch/aarch64/encode/tests.rs

A function whose compiled body is large enough that a **conditional** branch must
span more than ±1 MiB fails native codegen on AArch64 with:

```
error: AArch64 branch 'b.eq' displacement 30948256 to 'if_else_32' exceeds ±1 MiB
```

The ±1 MiB reach is an architectural fact — AArch64 `B.cond` encodes its target
as a 19-bit signed word offset (`imm19` → ±2¹⁸ words × 4 = ±1 MiB). But
**rejecting a function that needs a farther conditional branch is a compiler
limitation, not an architectural one.** The standard, well-understood fix is
*branch relaxation / veneer insertion*: when a `b.<cond> far` is out of range,
invert the condition and skip over an unconditional `b` (which reaches ±128 MiB
via `imm26`): `b.<!cond> skip ; b far ; skip:`. GCC, LLVM, and ld all do this
routinely. This backend has no such pass and errors instead, so a legal MFBASIC
program stops compiling once one function gets big. The single correct behavior
a fix produces: **such a function compiles and runs correctly**, with in-range
branches byte-identical to today.

The failure is easy to hit because MFBASIC constructs are code-heavy — a single
`WITH rec { … }` over a record with a `List` field lowers to a whole-record
rebuild (~3.8 KB of code in the repro below), so a function of only a few
thousand such statements — or a few hundred lines of ordinary dense code —
crosses 1 MiB. It was hit for real while building `examples/ai_chat`: a ~700-line
`main` with a large inline keypress ladder overflowed the loop-exit branch, and
`main` had to be split into helper functions (`handleKey`/`handleModal`/…) purely
to dodge the missing veneer.

References:

- `src/arch/aarch64/encode/emitter.rs:patch_labels` — the check that errors.
- `bugs/bug-124` (cited in that code) — chose to *reject* an out-of-range
  displacement rather than mask it to a wrong target. Correct as far as it goes;
  this bug is the missing next step (relax instead of reject).
- `src/ir/tests.rs:6224` (bug-401) — the same failure mode from an exponential
  `TRAP` lowering; fixed by making lowering linear (keep the function small),
  i.e. the de-facto contract has been "stay under 1 MiB" rather than "support
  long branches".
- Found during the `examples/ai_chat` task; memory note
  `mfb-large-function-branch-range`.

## Failing Reproduction

Generate a function with one `IF` whose then-body exceeds 1 MiB of code, so the
IF-false skip branch is out of range:

```
python3 - <<'PY'
N = 8000
L = ["TYPE R", "  a AS Integer", "  xs AS List OF Integer", "END TYPE",
     "FUNC main() AS Integer", "  MUT r AS R = R[0, []]", "  IF r.a > 0 THEN"]
L += ["    r = WITH r { a := r.a + 1 }"] * N
L += ["  END IF", "  RETURN r.a", "END FUNC"]
open("src/main.mfb","w").write("\n".join(L)+"\n")
PY
mfb build <project>
```

- Observed: `error: AArch64 branch 'b.eq' displacement 30948256 to 'if_else_32' exceeds ±1 MiB` (build fails, no executable). Compile ~62 s with the debug `mfb`.
- Expected: builds and runs; `main` returns 0 (the `IF` body is skipped at runtime).

Contrast cases that work today (bound the bug):

- The identical program with a body **under** ±1 MiB compiles and runs — the
  branch fits in `imm19`.
- An **unconditional** `b` reaches ±128 MiB (`imm26`), so straight-line/loop-back
  code of the same size is fine; only a *conditional* branch spanning >1 MiB
  fails. This is exactly what the veneer exploits.

| Environment | arch | Result |
| --- | --- | --- |
| macOS | aarch64 (debug `mfb`) | fails ✗ |

Arch-specific by nature (the branch encoding). riscv64's conditional reach is
even shorter (±4 KiB, per `src/arch/riscv64/encode/mod.rs:18`), so that backend
needs the same relaxation and may already have it or hit this sooner — audit
separately.

## Root Cause

`src/arch/aarch64/encode/emitter.rs:patch_labels` resolves each branch's target
and range-checks the displacement:

```rust
let (limit, span) = if patch.kind == "b" { (1<<27, "±128 MiB") }   // imm26
                    else                  { (1<<20, "±1 MiB")  };   // imm19 (b.<cond>)
if delta < -limit || delta >= limit {
    return Err(format!("AArch64 branch '{}' displacement {delta} to '{}' exceeds {span}", ...));
}
```

When a conditional branch (`b.eq`/`b.ne`/… — the only conditional kinds the
backend emits; there are no `cbz`/`tbz` patch kinds) is out of `imm19` range, it
returns a hard error. There is **no branch-relaxation/veneer pass** anywhere in
the AArch64 encode path — `patch_labels` is the only place displacements are
considered, and it only validates, never rewrites. So a valid program with a
large function is refused. The check itself is correct (better than bug-124's
prior mask-to-wrong-target); what is missing is the relaxation that keeps every
`imm19` in range so the check never trips for a conditional.

## Non-goals

- **Do not** mask an out-of-range displacement to a wrong target — that is the
  bug-124 regression this check was added to prevent. The check stays as a
  last-resort guard (and still legitimately fires for an unconditional `b` past
  ±128 MiB, a far larger and out-of-scope case).
- **Do not** weaken the encoder-helper range tests
  (`src/arch/aarch64/encode/tests.rs:1261-1262`: `branch_imm19(0, 1<<21).is_err()`
  etc.) — the raw encoders must still reject overflow; the fix prevents the
  relaxation pass from ever handing them an out-of-range value.
- **Do not** change the encoding of, or emit any extra instruction for, a branch
  that is already in range — in-range output must stay byte-identical (golden /
  `.ncode` neutral).
- **Do not** "fix" this by capping function size or documenting a limit; the
  program is legal and must compile.

## Blast Radius

- Every conditional branch kind the backend emits — `b.eq b.ne b.ge b.lt b.gt
  b.le b.vc b.vs b.hi b.lo b.mi b.ls` (`emitter.rs:204-215`) — shares the one
  code path and is fixed by the one relaxation pass (condition inversion is
  uniform: each has a complementary condition).
- Unconditional `b` past ±128 MiB — still errors; genuinely out of scope (no
  real function is 128 MiB), left as-is.
- riscv64 backend (`src/arch/riscv64/encode`) — conditional reach ±4 KiB; same
  class of limitation, audit and likely needs the same pass. Out of scope here;
  note it, don't silently ignore.
- Existing callers that split functions to stay small (e.g. `examples/ai_chat`)
  are unaffected by the fix (they already fit); they simply would not have needed
  to.

## Fix (phased, test-first)

1. **Failing tests + audit (no behavior change).**
   - An encoder-level unit test in `src/arch/aarch64/encode/tests.rs`: emit a
     `b.eq` to a label placed >1 MiB away (pad with data/instructions), run the
     encode, and assert it now produces a correct relaxed sequence (inverted
     `b.ne` over an unconditional `b`) that lands on the target — instead of
     erroring. Fast; this is the primary guard.
   - An end-to-end `tests/rt_aarch64_conditional_branch_relaxation.rs` that
     builds a program forcing an out-of-range conditional and asserts it compiles
     and returns the right value. NB: this compiles a ~1 MiB function (~minute);
     gate it as a slow/opt-in test so it does not bloat the default suite.
   Both must fail today. Commit:
2. **Add a branch-relaxation pass.** After layout but before/within final
   encoding, detect each conditional branch whose target is out of `imm19` range
   and rewrite it: `b.<cond> far` → `b.<!cond> skip ; b far ; skip:` (or route
   through a trampoline island if `far` is itself >128 MiB, which for a
   conditional it never is). Because inserting the extra `b`+label shifts
   downstream offsets and can push other branches out of range, iterate layout to
   a fixpoint (the standard branch-relaxation loop), or reserve worst-case slack
   and settle. Keep in-range branches untouched so their bytes are identical.
   Commit:
3. **Verify byte-neutrality + full suite.** Confirm no golden/`.ncode` drift for
   any program that has no out-of-range branch (the pass is a no-op for them).
   Run full `cargo test` and the acceptance harness. Commit:

## Notes

- Workaround with no compiler change (used by `examples/ai_chat`): split a large
  function into smaller ones so no single function's conditional branch spans
  >1 MiB. Bundle non-resource state into a record and make helpers pure so the
  split is clean (a record may not hold a resource, so the live child map stays
  in the caller).
