# bug-550: every `debug_assert!` in the tree is decorative — CI builds release, so none of the 55 ever runs

Last updated: 2026-09-05
Effort: small for the CI job; medium if load-bearing assertions are promoted
Severity: MEDIUM (missing gate; it has already cost one miscompile)
Class: Missing gate / test infrastructure

Status: **FIXED** (2026-09-06, `157a8dc52`)


## USER DECISION (2026-09-06) — promote, do not add a CI axis

Ruling: **promote the load-bearing `debug_assert!`s to unconditional `assert!`.**
No debug-assertions CI job.

So the work is an AUDIT of all 55, not a workflow change, and each promotion has
to be justified one at a time. Two constraints that fall out of the choice:

- An `assert!` that runs in release is on the hot path in a way a
  `debug_assert!` never was. A promotion whose predicate is expensive (anything
  walking a collection or re-deriving a type) is NOT a candidate — it changes
  compile time for every user. Say so per assert rather than promoting it.
- The remainder stay dead by decision, not by oversight. Record which ones were
  left and why, so the next audit does not re-litigate all 55.

`raise_error_bare`'s error-list check is the specific one bug-553 needs, and is
the first candidate to examine.

## The finding

    $ grep -rn 'debug_assert' src/ | wc -l
    55
    $ grep -nE 'cargo (test|build)' .github/workflows/coverage.yml
    50:  run: cargo build --release --bin mfb
    168: run: cargo test --release --workspace --no-fail-fast
    173: run: cargo test --release --workspace --target x86_64-unknown-linux-musl --no-fail-fast

`coverage.yml` is the only workflow. Every job builds and tests **release**, and
`debug_assertions` is off in release, so all 55 assertions are compiled out on
every platform CI covers. They are documentation that looks like enforcement.

This is the same class as the four dead-enforcement findings of 2026-09-05
(bug-470's lock covering 3 of 11 writers; `curve448_secret_paths_are_branch_free`
covering one of two curve fields; `test-winapp.sh` covering one of two halves of
itself; bug-491's pin check whose callers discarded its `Result`) — with the
sharpest variant: here the check is *written and correct* and simply never runs.

## It has already cost a miscompile

`strings::left`/`right`/`padLeft`/`padRight` raised `ErrInvalidArgument` while
declaring `errors: vec![]`. `inline_builtin_is_infallible` therefore proved them
infallible, the compiler emitted `TYPE_INLINE_TRAP_DEAD_HANDLER`, **deleted the
live handler**, and the program aborted with `7-705-0002` instead of recovering.

The invariant that would have caught it is encoded: `raise_error_bare` is meant
to assert that a raised error appears in the member's `errors:` list. That
assertion is a `debug_assert!`. It has never executed in CI, so four members
carried the wrong declaration until an inline-`TRAP` fixture found them by
behaviour (fixed 2026-09-05, `6d9f4b79a`).

## Reproduction

Not a runtime bug; the reproduction is the two commands above. To confirm an
individual assertion is inert, build release and exercise the path it guards —
e.g. before `6d9f4b79a`, a `strings::padLeft` with a negative width raised
without tripping the declaration assertion.

## The decision

Two fixes, and they are not exclusive:

1. **Add a debug-assertions CI job.** Cheapest, and it turns all 55 on at once.
   Cost: a second `cargo test` matrix entry; the suite is already the long pole in
   CI, and a debug build of this compiler is substantially slower than release,
   so this is a real wall-clock decision rather than a free one. It also risks a
   burst of pre-existing failures the first time it runs — which is information,
   but has to be budgeted for.
2. **Promote the load-bearing ones to real `assert!`.** Targeted: the ones that
   encode a compiler invariant whose violation is a miscompile (the
   `raise_error_bare` declaration check is the proven example) become
   unconditional. Cost: a release-mode branch on a hot path for some of them, and
   someone has to classify all 55.

A reasonable split is (2) for the handful that guard miscompiles and (1) for
everything else, but that is a judgement about CI budget and hot-path cost, so it
is recorded here rather than assumed.

## Blast radius

All 55 sites; `src/optimizer/`, `src/ir/`, `src/codegen/`, `src/target/` and the
canvas backends each hold some. Turning them on for the first time will surface
whatever has rotted behind them — expect the first run to be informative rather
than green.

References: `.github/workflows/coverage.yml` (the only workflow);
`src/codegen/builtins/mod.rs:inline_builtin_is_infallible`;
`bugs/completed/bug-533-*` and the `6d9f4b79a` commit (the two most recent
dead-handler miscompiles); `.ai/testing-gates.md`.

Found while landing `6d9f4b79a` — the fix's root-cause analysis named the
`debug_assert!` as the reason the invariant was unenforced, which generalizes
past that bug.

## The audit (2026-09-06) — all 43 debug-only sites classified

The document's headline count of 55 is a loose grep that counted prose mentions.
The real population, on `31ccac1cf`:

    $ grep -rn "debug_assert!(\|debug_assert_eq!(\|debug_assert_ne!(" src --include='*.rs' | grep -v '///' | wc -l
    35
    $ grep -rn "cfg(debug_assertions)" src --include='*.rs' | wc -l
    8

**35 macro sites + 8 `cfg` gates = 43.** Every one is classified below; the
count after the change is 12 macro sites + 5 gates.

### Promoted to `assert!` — 23 sites, 4 gates

Each is O(1), or bounded by the register file / a single pass, and each guards a
failure that is silent or non-local:

| site | guards |
| --- | --- |
| `builder_error_emission.rs:22` | a builtin raising an error it does not declare — the exact data `inline_builtin_is_infallible` reads, and the cause of three dead-handler MISCOMPILES this month |
| `vreg_frame.rs:128,512` (gate) | bug-360: an sp-relative access past the frame, i.e. a write over the CALLER's stack |
| `linear_scan.rs:697` (gate) | bug-54: a callee-saved register written by generated code and missing from the frame save set |
| `link_thunk.rs:1647` | sec-01: `FREE` on an `AS RES` producer, i.e. freeing a live handle |
| `simplifycfg.rs:146` | a block stream whose last instruction is not a terminator |
| `ir/lower.rs:2144` | the `TRAP`-hoist scan and rewrite disagreeing on which nodes lift |
| `lencache.rs:322` | a len-cache bind with fewer than two readers (a partial rewrite) |
| `pe.rs:195,343`, `windows/link/mod.rs:368,385` | four PE layout invariants — and **no Windows binary this repo builds is ever executed by a test**, so these are close to the only dynamic check the PE writer has |
| `squashfs/mod.rs:195,196,456` | a zero-length metadata block (the kernel answers `-EIO`) and the superblock size |
| `os/note.rs:52` | the note descriptor length |
| `byte_list.rs:131` | the allocation-return-register contract |
| `syscall_io.rs:230` | bug-467: an EPIPE-classifying write site with no errno accessor, which turns `prog \| head` into an `ErrWriteFailed` |
| `float_parse_table.rs:156,193` | bignum subtraction going negative, and a quotient wider than 128 bits |
| `metal.rs:906`, `vulkan.rs:2178` | one pipeline per `BlendMode` — the frame path indexes this array with no bounds check |
| `registry/mod.rs:1861` | a registry row name disagreeing with its lookup |
| `validation.rs:442,635,664` (gates) | plan-111-C: a type-key MERGE (two spellings collapsing, so a lookup returns the wrong record layout, union tag or close op) or SPLIT (a hit becoming a miss). Both silent. Cost is bounded by the number of distinct TYPES and paid twice per build, not per function |

### Converted to a `const` assertion — 2 sites

`metal.rs:2608` and `vulkan.rs:4019` both assert `ITEM_BLOCK_SIZE % 8 == 0`.
Both operands are compile-time constants, so `const _: () = assert!(…)` decides
this when the COMPILER is built and costs nothing at run time — strictly better
than either spelling.

### Covered by a new test instead — 7 sites

`registry/mod.rs`'s `add_record`/`add_union`/`add_enum`/`add_function`/
`add_constant` and `Body::mfb`/`mfb_with_fast_path` all assert the SHAPE of
static registry data. The right instrument is a test that walks the built
registry once, not a check paid on every user's compile — and that is already
this file's own documented precedent (plan-116-E **E6**, beside
`the_consuming_parameters_name_real_members`). The asserts stay at the
construction site, where they give the sharper message; the new
`every_registry_row_has_the_shape_its_builder_requires` is what actually runs.

### Left debug-only, with the reason — 5 sites, 4 gates

So the next audit does not re-litigate them:

- `regalloc/mod.rs:257` — **expensive.** It renders EVERY field of EVERY
  instruction to a `String` and parses each as a vreg: O(instructions × fields)
  with an allocation per field. This is the case the ruling names. It could be
  made cheap by testing the operand without rendering; that is a rewrite, not a
  promotion.
- `riscv64/v128.rs:178` (gate) — **expensive**, O(n²) over the slot map.
- `x86_64/select.rs:204` — a `debug_assert!(false)` with a *deliberate* release
  fallback and a written rationale (map the residual token to the call bank so it
  still ENCODES). Promoting would replace a documented degradation with a panic;
  that is a design change needing its own evidence, not an audit call.
- `rules/mod.rs:270` and its `:342` gate — same shape: the release path emits a
  VISIBLE `0-000-0000 UNKNOWN_RULE` sentinel. A panic is worse for a user than a
  diagnostic that names itself. The right instrument here is a test that checks
  every emit site's rule name against `RULES`; that is worth doing and is not
  this change.
- `ir/link.rs:97` — asserts the target is not `arm64_32`/`riscv32`. **No such
  target exists**, so it can never fire; promoting it is pure cost for zero
  coverage. It is documentation of a future hazard and reads correctly as one.
- `float_parse_ref.rs:894` — **not in the population.** The module is
  `#[cfg(test)]`; it is an oracle and never ships.

### The cost, measured

"An expensive predicate is not a candidate" is only a rule if somebody checks.
Interleaved A/B on `examples/browser/app` at `-O3`, n=7 each, alternating so both
binaries see the same machine load:

    BASE  min 11.969s   median 12.313s
    MOD   min 11.819s   median 12.173s
    delta min -1.25%,   median -1.14%

The promoted build is *faster* on both statistics — which is to say the added
work is below the noise of a 12-second compile. Interleaving matters: an earlier
non-interleaved run on a loaded box made the same change look like +21%.

A first attempt at this measurement was worthless and is worth recording. It
reported `rc=1` for both binaries at 0.05s a build: `examples/browser/app`
imports three sibling packages that must each be built to a `.mfp` and installed
first, so nothing was ever compiled. **A benchmark that never ran the work reads
exactly like a fast one** — check the exit status before reading the number.

### Coverage — did any of them fire?

No. Across `cargo test --release --no-fail-fast` (148 binaries) and the 1562
compiles the artifact gate performs, none of the 23 promoted assertions and
neither ungated block tripped. `test-accept.sh` 1418 ran, artifact gate 1941
golden(s) / 0 diff(s).

The new registry test was checked for vacuity rather than assumed: it walks
**105 records, 8 unions, 22 enums, 123 constants, 587 functions and 363 `Mfb`
bodies**, and an injected rewrite-target typo reds it naming `crypto.ulid`. A
shape test that iterates nothing passes for free and reads exactly like one that
works.
