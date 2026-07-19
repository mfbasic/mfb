# bug-322: arena-allocation / internal-call / error-tail emission is open-coded ~1,500 lines across `shared/code`, with no `CodeBuilder` helper

Last updated: 2026-07-19
Effort: large (3h–1d)
Severity: LOW
Class: Other (cleanup / duplication)

Status: Fixed (2026-07-19). Zero inline arena relocation literals remain in
`src/target/shared/code/` (`grep -c 'ARENA_{ALLOC,FREE}_SYMBOL.to_string()'` -> 0).
Landed: 3 `internal_reloc` + 5 `emit_alloc` copies deleted in favour of the
pre-existing `internal_branch` / one shared `emit_alloc`; `emit_arena_alloc_call`
and `emit_arena_free_call` added and adopted at every CodeBuilder site (45 exact
+ 5 indentation variants + 11 free/alloc); 33 remaining inline `CodeRelocation`
literals routed through `internal_branch`. Gate: artifact-gate 1,189 goldens,
0 diffs, at every step.

Three sites are deliberately NOT folded into the alloc helper, and this is a
finding rather than a deferral: `entry_and_arena.rs` `_mfb_simd_alloc_list`
(:1386) and `_mfb_build_error_loc` (:1477) already share `internal_branch`, but
their *failure tails* are genuinely different — the first uses a status-register
protocol (tag to RET[1], zero x0, branch) and the second returns a null pointer
because it IS the ErrorLoc builder, so giving it an error-return path would
recurse. `fs_helpers_io.rs`'s `alloc_call` closure likewise already uses
`internal_branch`; only its reloc/instruction push order differs, which is
immaterial since the two vectors are independent.

The "107 error tails" this document counts are not duplication: they are 107
call sites of a single shared `emit_fail` (`tls/mod.rs:194`). Nothing to extract.

The single largest duplication cluster in the codebase. Three independent
reviewers (Agent 01 collection builders, Agent 02 string/conversion builders,
Agent 07 tls/crypto/link codegen — plus corroboration from Agents 04, 05, 06,
08) converged on the same finding from different directories: the
"call `_mfb_arena_alloc`, push the call relocation, compare the result tag,
branch, emit an error tail" idiom is written out by hand at every allocation
site in `src/target/shared/code/`, and there is no shared emitter for it.

There is no `CodeBuilder::emit_arena_alloc` anywhere in the tree. In its place
six ad-hoc free-function copies have accreted in the leaf backends
(`crypto.rs`, `crypto_ec.rs`, `crypto_ec/openssl.rs`, `net/mod.rs`,
`tls/mod.rs`, `link_thunk.rs`), each with the same four-parameter signature,
none visible to the 45 `CodeBuilder`-method sites that need it. A seventh
partial helper — `CodeBuilder::emit_symbol_call` — *does* exist and does
exactly the `branch_link` + relocation push half of the job, but has only
**4 call sites in the entire repository**, three of which are in its own file.

The single correct outcome a fix produces: one shared helper set on
`CodeBuilder` (plus a free-function twin for the non-`CodeBuilder` dialect),
adopted at every site, emitting **byte-identical machine code and relocations**
to what is emitted today. This is a readability/maintenance fix, not a
behavior fix — nothing about the compiled binary may change.

References:

- `/tmp/cleanup-findings/index.md` — Agent 01 #2, Agent 02 #4, Agent 07 #1 and
  #4, Agent 08 #2, Agent 06 #8 and #12, Agent 05 #4 and #7, Agent 04 #7.
- `src/target/shared/code/builder_emit_helpers.rs:4` (`emit_symbol_call`, the
  existing-but-unadopted half of the fix).
- `src/target/shared/code/builder_codegen_primitives.rs:298,375,767`
  (`emit_allocation_error_return`, `emit_error_code_return`,
  `emit_error_register_return` — the existing error-emission cluster).
- Found during the cleanup-focused source review at base `25c38ba1`.

## Audit addendum (2026-07-19, re-measured at HEAD `4e0b6e04d`)

The measurements below were re-verified 69 commits after the base this document
was written against (`25c38ba1`). The headline figures held; these did not.

**Corrections.**

- **The shared `internal_reloc` twin already exists and is already adopted.**
  Fix Design §2 says the three `internal_reloc` copies "should be promoted to
  one shared definition rather than reinvented." Backwards: `internal_branch`
  (`src/target/shared/code/mod.rs:2720`) is already that definition — field-for-
  field identical, same parameter order — with 59 call sites across 9 files.
  The three copies (`net/mod.rs`, `tls/mod.rs`, `link_thunk.rs`) were pure
  redundancy and are **deleted, their 4 call sites routed to
  `super::internal_branch`** (done 2026-07-19; all three are child modules of
  `code`, so the private fn was already in scope).
- `builder_strings_builtins.rs:1444,1451,1467` (Open Decisions) are **wrong** —
  `:1444` is `self.emit(abi::label(&alloc_ok))`. There is exactly **one**
  suspect site in that file, `:1485-1486`.
- Phase 4's `io_helpers.rs:751-762` is the **wrong range**; the hand-rolled
  `adrp`/`add_pageoff` pair is `:774-784`.
- Counts that drifted at HEAD: `ARENA_FREE_SYMBOL` 13 → **14**;
  `RelocIntent::Call` 129/150/160 → **131/152/162**.
- `grep -rn 'fn emit_alloc' src/` returns **8**, not 6 — it also catches
  `audio/{macos,alsa}.rs`'s `emit_alloc_byte_list`, which are **not** the
  pattern. The six named copies are the right set.
- Three more `push_symbol_address` clones the census missed: `tls/mod.rs:158`,
  `crypto_ec/macos.rs:78`, `crypto_ec/openssl.rs:185`.

**Sites that must NOT be converted** (found by reading, not listed in Blast Radius):

- `entry_and_arena.rs:1392` (`_mfb_simd_alloc_list`) and `:1483`
  (`_mfb_build_error_loc`) — the relocation is declared ~58 lines later in a
  separate `vec![]`, and neither failure tail is an error return: the first uses
  a status-register protocol, the second returns a null pointer. Converting
  `:1483` would inject an error-return path into the ErrorLoc builder itself.
- `entry_and_arena.rs:1354` — the reloc *inside* `_mfb_arena_alloc`'s own body.
- `fs_helpers_io.rs:1043` — a local `alloc_call` closure that pushes the
  relocation **before** the instruction, inverting the ordering every other site
  uses.

**The size-overflow hazard is real, and the suspect set is 3, not 5:**
`builder_strings.rs:217-218`, `:462-463`, `builder_strings_builtins.rs:1485-1486`.
The chain is confirmed: `emit_checked_size_add_immediate` leaves the *wrapped*
size in `x0`, `emit_allocation_error_return` reaches
`emit_error_register_return(RESULT_TAG_REGISTER, …)`, and
`RESULT_TAG_REGISTER == abi::RET[0] == x0` — so the error *code* is a garbage
size. All 34 other allocation overflow labels route to
`emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, …)` correctly. **Caveat before
filing:** all three carry a comment citing bug-60 claiming the shared error is
deliberate, so check bug-60 before treating this as an oversight.

**Load-bearing precondition for output-neutrality, asserted but never evidenced
in this document:** adopting `emit_symbol_call` at arena sites is neutral only
because it yields `("internal", None)` exactly when `platform_imports` misses
the symbol, and `_mfb_arena_alloc`/`_mfb_arena_free` are never
`platform_imports` keys. Add a test asserting that before Phase 2, and require
the artifact gate to show zero `.nobj` diff **including relocation-table order**
— label-counter drift is the likeliest silent breaker.

## Current State

All counts below were re-measured against the worktree at `25c38ba1`; each
differs in at least one respect from the originally-reported lead, and the
corrections are noted inline.

| Measurement | Command | Count |
| --- | --- | --- |
| `branch_link(ARENA_ALLOC_SYMBOL)` sites, `shared/code/` | `grep -rc` | **124** |
| `branch_link(ARENA_FREE_SYMBOL)` sites, repo-wide | `grep -r` | **13** |
| `RelocIntent::Call` occurrences, `shared/code/` | `grep -r` | **129** |
| `RelocIntent::Call` occurrences, `src/target/` | `grep -r` | **150** |
| `RelocIntent::Call` occurrences, repo-wide | `grep -r` | **160** (10 in `src/arch/` are reloc-kind mapping, not pushes) |
| Byte-identical 12-line alloc-and-check blocks | exact substring match | **45** |
| `emit_fail(...)` call sites, tls + crypto_ec | `grep -o` | **77** (exact) |
| `emit_fail(...)` call sites, audio | `grep -c` | **30** (15 in `macos.rs`, 15 in `alsa.rs`) |
| Ad-hoc alloc free-function copies | `grep -rn 'fn emit_alloc'` + `emit_arena_alloc` | **6** |
| `internal_reloc` free-function copies | `grep -rn 'fn internal_reloc'` | **3** |
| `CodeBuilder::emit_symbol_call` call sites | `grep -r` | **4** |
| `CodeBuilder::emit_arena_alloc` | `grep -rn 'fn emit_arena_alloc'` | **0** (the only `emit_arena_alloc` is a free fn at `crypto.rs:242`) |

### Corrections to the reported leads

- Agent 01 #2 reported "50 byte-identical arena-alloc blocks". **45** blocks
  match the canonical 12-line form byte-for-byte. The broader family is larger
  than 50 (124 `branch_link(ARENA_ALLOC_SYMBOL)` sites), but only 45 are
  literally identical; the remainder are the free-function dialect (below).
- Agent 01 #2 reported "127 `RelocIntent::Call` pushes". The true figure is
  **129** in `shared/code/`, 150 across `src/target/`, 160 repo-wide.
- Agent 02 #4 reported "5 ad-hoc local `emit_alloc` free functions". There are
  **6**, and one is misnamed relative to the lead: `crypto.rs:242` is
  `fn emit_arena_alloc`, not `fn emit_alloc`. The sixth,
  `crypto_ec/openssl.rs:376`, is a redundant re-declaration of its own parent's
  `crypto_ec.rs:241` (independently caught as Agent 07 #11).
- Agent 07 #4 reported "~77 near-identical label+`emit_fail` blocks". **Exactly
  77**, distributed 27 / 27 / 13 / 10 as reported.
- Agent 02 #4's "~150 repo-wide" estimate is not reproducible as stated; the
  defensible repo-wide figure is 124 arena-alloc call sites.

### The canonical duplicated block (verbatim)

`src/target/shared/code/builder_collection_mutate.rs:588-606`:

```rust
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
```

The identical 19 lines appear again at `builder_collection_mutate.rs:980-998`
and `:1361-1379` (verified character-for-character by `sed`-extract and
compare). The 12-line core (`branch_link` through the `compare_immediate`
closing paren) recurs 45 times:

- `builder_strings_builtins.rs` — 10
- `builder_collection_mutate.rs` — 10
- `builder_collection_layout.rs` — 7
- `builder_strings.rs` — 6
- `builder_arena_transfer.rs` — 4
- `builder_search.rs` — 2, `builder_codegen_primitives.rs` — 2
- `builder_fs_paths.rs`, `builder_collection_queries.rs`,
  `builder_inplace_assign.rs`, `builder_value_semantics.rs` — 1 each

### The second dialect

The `fs_helpers_*`, `net/`, `tls/`, `crypto*`, `link_thunk` and `audio/`
modules are **not** `CodeBuilder` methods — they are free functions that thread
`&mut Vec<CodeInstruction>` and `&mut Vec<CodeRelocation>` explicitly. The
same idiom there reads (`fs_helpers_paths.rs:37-51`):

```rust
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
```

This dialect accounts for the 18 sites in `fs_helpers_paths.rs`, 11 in
`fs_helpers_atomic.rs`, 10 in `fs_helpers_io.rs`, and 8 in `os.rs`. **A
`CodeBuilder` method alone cannot serve them** — this is the single most
important structural constraint on the fix.

## Root Cause

No architectural cause; this is accretion. `CodeBuilder::emit_symbol_call`
(`builder_emit_helpers.rs:4`) was written to be exactly the shared
`branch_link`-plus-relocation emitter and then never adopted — 4 call sites
against 129 `RelocIntent::Call` occurrences in the same directory. Each new
builder module copied the nearest neighbor rather than reaching for it, and
because the leaf backends live in the free-function dialect they could not
have used it even had they known about it. Six of them independently grew
their own private copy (`crypto.rs:242`, `crypto_ec.rs:241`,
`crypto_ec/openssl.rs:376`, `net/mod.rs:45`, `tls/mod.rs:191`,
`link_thunk.rs:109`), all with the same
`(symbol, &mut instructions, &mut relocations, fail)` signature, alongside
three copies of `internal_reloc` (`net/mod.rs:32`, `link_thunk.rs:69`,
`tls/mod.rs:142`).

## Goal

- One `CodeBuilder` helper set covers every arena-allocation, internal-call,
  arena-free, and error-tail emission in `src/target/shared/code/`, with a
  free-function twin for the non-`CodeBuilder` dialect.
- The 6 ad-hoc alloc copies and 3 `internal_reloc` copies collapse to one each.
- `cargo build` output is **byte-identical** before and after, proven by
  `scripts/artifact-gate.sh`.
- Net reduction on the order of 1,200–1,500 lines across `shared/code/`.

### Non-goals (must NOT change)

- **Emitted bytes.** Not one instruction, relocation, label name, or label
  *ordering* may change. This is the hard constraint; the artifact gate exists
  precisely to enforce it and must be run on every phase.
- Relocation ordering within a function. Several sites push the relocation
  *before* the following instructions and several after; a helper that
  normalizes the order changes the `.nobj` relocation table even when the code
  bytes match. Preserve per-site ordering or prove the table is unchanged.
- Label naming. `self.label(prefix)` is a counter; reordering helper calls
  renames labels and churns every descriptive golden.
- The **size-overflow error-code semantics** (see Open Decisions). A mechanical
  unification must not silently alter which error code a size-overflow path
  returns.
- Do NOT "validate" this work by regenerating goldens to match new output. If a
  golden shifts, the refactor is wrong. Goldens are the oracle here, not the
  artifact.

## Blast Radius

Every site below shares the pattern. Verdicts:

**In scope — `CodeBuilder` dialect (45 exact blocks, 124 alloc sites total):**

- `builder_collection_mutate.rs` (10 exact) — fixed by this bug
- `builder_strings_builtins.rs` (10 exact) — fixed by this bug
- `builder_collection_layout.rs` (7 exact) — fixed by this bug
- `builder_strings.rs` (6 exact) — fixed by this bug
- `builder_arena_transfer.rs` (4 exact) — fixed by this bug
- `builder_search.rs`, `builder_codegen_primitives.rs` (2 each) — fixed here
- `builder_fs_paths.rs`, `builder_collection_queries.rs`,
  `builder_inplace_assign.rs`, `builder_value_semantics.rs` (1 each) — fixed here
- `builder_values.rs` (4 alloc sites, non-canonical form) — fixed here
- `runtime_helpers.rs` (4), `entry_and_arena.rs` (3), `term_grid.rs` (2),
  `term.rs` (2), `io_helpers.rs` (5), `stdin_broadcast.rs` (1),
  `float_format.rs` (1) — fixed here

**In scope — free-function dialect (needs the free-fn twin):**

- `fs_helpers_paths.rs` (18), `fs_helpers_atomic.rs` (11),
  `fs_helpers_io.rs` (10), `os.rs` (8) — fixed by this bug
- `net/mod.rs:45`, `tls/mod.rs:191`, `crypto.rs:242`, `crypto_ec.rs:241`,
  `crypto_ec/openssl.rs:376`, `link_thunk.rs:109` — **deferred to a follow-up
  phase**, see Fix Design ordering constraint
- `net/mod.rs:32`, `link_thunk.rs:69`, `tls/mod.rs:142` (`internal_reloc`) —
  deferred with their `emit_alloc` siblings

**Error-tail sites, in scope for `emit_error_tails`:**

- `tls/openssl.rs` (27 `emit_fail`), `tls/macos.rs` (27),
  `crypto_ec/openssl.rs` (13), `crypto_ec/macos.rs` (10) — 77 total
- `audio/macos.rs` (15), `audio/alsa.rs` (15) — 30 total
- `net/io.rs` — 30 `alloc_fail` references, 6 `closed` tails

**Related, NOT in scope:**

- `push_error_message_address` (`data_objects.rs:34`) reimplementing
  `push_symbol_address` (`data_objects.rs:7`) — same cluster, but a distinct
  2-function collapse; see Fix Design.
- `push_log_address` (`stdin_broadcast.rs:38`) — third copy of the same shape,
  fold in with the above.
- `src/arch/*/reloc.rs` `RelocIntent::Call` occurrences (10) — unaffected;
  these are the reloc-kind *mapping*, not pushes.
- Agent 03 #2's array-kernel driver prologue and Agent 04 #1's
  `lower_runtime_helper` `CodeFunction` literal — adjacent duplication clusters
  with their own root causes; separate bugs.

## Fix Design

### The helper set

Four helpers, each with a `CodeBuilder` method and (where the free-function
dialect needs it) a free-function twin taking `&mut Vec<_>` pairs:

1. **`CodeBuilder::emit_arena_alloc(&mut self, size_reg, align, ok_label, fail: AllocFail)`**
   — emits the `move_immediate(ARG[1], align)` / `branch_link` / relocation
   push / `compare_immediate(return_register(), RESULT_OK_TAG)` /
   `branch_eq(ok_label)` sequence, then the failure tail selected by
   `AllocFail`. Replaces the 45 exact blocks and the 79 near-variants.

2. **`emit_internal_call` — already exists as `CodeBuilder::emit_symbol_call`
   (`builder_emit_helpers.rs:4`).** Do **not** write a new one. It performs the
   `branch_link` plus the relocation push and correctly derives
   `binding`/`library` from `self.platform_imports`, which for internal symbols
   such as `ARENA_ALLOC_SYMBOL` yields `("internal", None)` — byte-identical to
   the hand-written push. The work is **adoption**, from 4 call sites to ~129.
   A free-function twin is still needed for the `fs_helpers_*` / `os.rs`
   dialect; `internal_reloc` (3 copies) is that twin and should be promoted to
   one shared definition rather than reinvented.

3. **`CodeBuilder::emit_arena_free(&mut self, ptr_reg)`** — 13 sites repo-wide.
   Small, but it is the symmetric counterpart and there is already a
   free-function `emit_arena_free` at `tls/mod.rs:208` proving the shape.

4. **`emit_error_tails(&mut self, tails: &[(label, code, msg_symbol)])`** — a
   table-driven epilogue emitter. Every one of the 77 tls/crypto and 30 audio
   `emit_fail` sites, and the `net/io.rs` alloc/closed/timeout tails, is a
   `label` followed by a fixed 3–5 instruction error return. Emitting them from
   a slice literal at the bottom of each lowering function is, per Agent 07 #4,
   "the largest mechanical reduction available" — and it is the one that most
   reduces the risk of a tail drifting out of sync with its label.

### Where it lives — recommendation

**`builder_codegen_primitives.rs`**, not `builder_emit_helpers.rs`.

Reasoning: `builder_codegen_primitives.rs` already owns the entire
error-emission cluster — `emit_error_code_return:375`,
`emit_allocation_error_return:298`, `emit_error_register_return:767`,
`emit_checked_size_multiply:318`, `emit_checked_size_add:335`,
`emit_checked_size_add_immediate:349`, `emit_build_error_loc:395`. The new
`emit_arena_alloc` and `emit_error_tails` call directly into those and belong
beside them. `builder_emit_helpers.rs` is only 525 lines and, per Agent 04 #18,
29% of it (`:360-511`) is a thread-transfer special case that does not belong
there at all — it is the less coherent home.

The counter-argument is real and should be recorded: `emit_symbol_call` already
lives in `builder_emit_helpers.rs`, so splitting the set across two files is
itself a smell. The recommendation is therefore: put the new helpers in
`builder_codegen_primitives.rs`, and **leave `emit_symbol_call` where it is**
for now — moving it would touch the diff surface for no output-visible gain.
Reconcile the two files during the `builder_codegen_primitives.rs` split that
Agent 04 #3 proposes (2,437 lines / 6 concerns), where an `error_emission.rs`
becomes the obvious joint home.

### Ordering constraint (required)

The 6 ad-hoc free-function copies (`crypto.rs:242`, `crypto_ec.rs:241`,
`crypto_ec/openssl.rs:376`, `net/mod.rs:45`, `tls/mod.rs:191`,
`link_thunk.rs:109`) and the 3 `internal_reloc` copies must be retired in a
**separate, later phase** — not the same one that introduces the shared
helpers. Two reasons:

1. Those six live in the free-function dialect and each carries a slightly
   different failure tail (`fail` label vs. inline error-code stores vs.
   `internal_reloc` vs. an inline `CodeRelocation` literal). Collapsing them is
   a genuine behavioral-equivalence argument per site, not a mechanical
   substitution.
2. Landing them together makes the artifact-gate diff unreadable. If the gate
   reports a byte difference in a combined phase, bisecting it across ~200
   edited sites in 30 files is far more expensive than across one dialect at a
   time.

Phase 2 does the `CodeBuilder` dialect. Phase 3 does the free-function dialect
in `fs_helpers_*` / `os.rs`. Phase 4 retires the 6 ad-hoc copies. Each gets its
own gate run.

### Rejected alternatives

- **A macro instead of a function.** Rejected: macros defeat the artifact gate's
  usefulness as a bisect tool (the expansion site is invisible in a diff) and
  `rustfmt` will not normalize the bodies, so drift returns.
- **Unifying the two dialects by converting `fs_helpers_*` to `CodeBuilder`
  methods.** Rejected for this bug: that is a much larger restructuring
  (Agent 06 #1 proposes a `code/fs/` directory) with its own risk profile.
  Provide the free-function twin instead.
- **Normalizing relocation push order while we are in there.** Rejected: it
  changes the `.nobj` relocation table and breaks the byte-identical guarantee
  that makes this refactor safe to land at all.

## Phases

### Phase 1 — audit + gate baseline (no behavior change)

- [ ] Capture a clean `scripts/artifact-gate.sh` baseline at HEAD; record the
      artifact hashes so every later phase compares against a known-good.
- [ ] Complete the blast-radius audit above; write a verdict per site into this
      file, distinguishing the two dialects.
- [ ] Resolve the size-overflow open decision (below) **before** writing the
      helper, since it determines the helper's signature.

Acceptance: baseline hashes recorded; every one of the 124 alloc sites and 107
error-tail sites carries a dialect verdict.
Commit: `—`

### Phase 2 — helpers + `CodeBuilder`-dialect adoption

- [ ] Add `emit_arena_alloc`, `emit_arena_free`, `emit_error_tails` to
      `builder_codegen_primitives.rs`.
- [ ] Adopt at the 45 exact blocks plus the `CodeBuilder`-dialect variants:
      `builder_collection_mutate.rs`, `builder_strings_builtins.rs`,
      `builder_collection_layout.rs`, `builder_strings.rs`,
      `builder_arena_transfer.rs`, `builder_search.rs`, `builder_values.rs`,
      `builder_collection_queries.rs`, `builder_inplace_assign.rs`,
      `builder_value_semantics.rs`, `builder_fs_paths.rs`, `term.rs`,
      `term_grid.rs`, `io_helpers.rs`, `runtime_helpers.rs`,
      `entry_and_arena.rs`, `stdin_broadcast.rs`, `float_format.rs`.
- [ ] Replace hand-written relocation pushes with `emit_symbol_call` at the
      same sites.

Acceptance: `scripts/artifact-gate.sh` reports **zero** artifact difference vs.
the Phase 1 baseline; `scripts/test-accept.sh` green with zero golden churn.
Commit: `—`

### Phase 3 — free-function dialect (`fs_helpers_*`, `os.rs`)

- [ ] Add the free-function twin; promote one `internal_reloc`.
- [ ] Adopt at `fs_helpers_paths.rs` (18), `fs_helpers_atomic.rs` (11),
      `fs_helpers_io.rs` (10), `os.rs` (8).

Acceptance: artifact gate zero-diff; acceptance suite green.
Commit: `—`

### Phase 4 — retire the 6 ad-hoc copies + the `push_*_address` trio

- [ ] Delete `crypto_ec/openssl.rs:376` outright (it re-declares its own
      parent's `crypto_ec.rs:241`).
- [ ] Route `crypto.rs:242`, `crypto_ec.rs:241`, `net/mod.rs:45`,
      `tls/mod.rs:191`, `link_thunk.rs:109` through the shared twin, one file
      per commit, gate-run between each.
- [ ] Collapse `push_error_message_address` (`data_objects.rs:34`) and
      `push_log_address` (`stdin_broadcast.rs:38`) into one-line wrappers over
      `push_symbol_address` (`data_objects.rs:7`). Note that
      `push_error_message_address` hand-builds `CodeInstruction::new("adrp")` /
      `("add_pageoff")` literals rather than calling `abi::load_page_address`
      (`src/target/shared/abi.rs:858`) and `abi::add_page_offset` (`:864`) —
      the emitted instruction is the same, but it bypasses the `abi` seam.
- [ ] Replace the 4 hand-rolled copies of that same sequence at
      `fs_helpers_paths.rs:52-63`, `fs_helpers_paths.rs:183-194`,
      `io_helpers.rs:468-479`, `io_helpers.rs:751-762` with calls — note
      `io_helpers.rs` already calls the real helper correctly 16 times
      elsewhere in the same file.

Acceptance: artifact gate zero-diff after each commit; full acceptance suite
green.
Commit: `—`

### Phase 5 — error-tail tables

- [ ] Adopt `emit_error_tails` at the 77 tls/crypto `emit_fail` sites and the
      30 audio sites.
- [ ] Adopt at the `net/io.rs` alloc/closed/timeout tails.

Acceptance: artifact gate zero-diff; acceptance suite green.
Commit: `—`

## Validation Plan

- **Regression test: `scripts/artifact-gate.sh`.** This is the whole safety
  argument. The gate is execution-free (~5 min) and compares generated
  artifacts byte-for-byte; a pure refactor must produce a zero-byte diff at
  every phase. Any non-zero diff means the refactor changed emitted code and
  must be reverted or explained, not accepted.
- **Full suite: `scripts/test-accept.sh`.** Must be green with **zero golden
  churn**. Golden movement is a failure signal here, not an expected outcome.
  Per the memory note, do not rebuild while the acceptance harness is running.
- **Runtime proof:** none required beyond the above — there is no behavior
  change to observe. That is the point.
- **Doc sync:** none expected. If Phase 4 changes the `abi` seam usage, check
  whether `spec/architecture/06_native.md` describes the relocation-emission
  path.
- **Line-count proof:** record `wc -l` on `src/target/shared/code/` before and
  after; the stated goal is a 1,200–1,500 line net reduction, and a materially
  smaller result means the adoption was incomplete.

## Open Decisions

- **The size-overflow / `emit_allocation_error_return` hazard — decide before
  writing the helper.** `emit_allocation_error_return`
  (`builder_codegen_primitives.rs:298`) is
  `emit_error_register_return(RESULT_TAG_REGISTER, ERR_ALLOCATION_MESSAGE)`,
  and `RESULT_TAG_REGISTER` is `abi::RET[0]` (`error_constants.rs:25`) — i.e.
  it reads the error code **out of x0**. `emit_error_register_return`'s own
  comment says so: *"the code may currently live in one of the other arg
  registers (the allocation path passes it in x0)"*
  (`builder_codegen_primitives.rs:775-777`).

  That is correct immediately after an `arena_alloc` call, where x0 holds the
  returned tag. It is **not** correct at a size-overflow label, which is
  branched to from `emit_checked_size_add` / `emit_checked_size_multiply`
  *before* any call — at which point x0 holds a partially-computed size.

  Both spellings exist in the tree today:

  - **Correct:** `builder_collection_mutate.rs:604` routes `size_overflow` to
    `emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)`.
  - **Suspect:** `builder_strings.rs:218` routes the `overflow` label to
    `emit_allocation_error_return()`, reached from
    `emit_checked_size_add(:184)` / `emit_checked_size_add_immediate(:199)`
    with no intervening call. Same shape at `builder_strings.rs:463` and, per
    Agent 02's incidental note, at `builder_strings_builtins.rs:1444,1451,1467`.

  A single `emit_arena_alloc` helper must pick one failure tail — and in doing
  so will either **fix** these sites (if it takes the `emit_error_code_return`
  path) or **entrench** them (if it takes the `emit_allocation_error_return`
  path). Neither may happen silently.

  **Recommendation:** treat this as a *separate correctness bug*, file it, and
  give `emit_arena_alloc` an explicit `AllocFail::{TagInRegister, Code(u32)}`
  parameter so each call site keeps exactly today's behavior and the refactor
  stays byte-identical. Fix the suspect sites in the follow-up bug, under a
  behavioral test, where the golden churn is intentional and reviewable. The
  alternative — unify to `emit_error_code_return` now — is tempting and
  probably correct, but it breaks the byte-identical guarantee that makes this
  large a refactor safe, and it would land a real behavior change under a
  cleanup banner.

- **Helper home.** `builder_codegen_primitives.rs` (recommended, co-located
  with the existing error-emission cluster) vs. `builder_emit_helpers.rs`
  (where `emit_symbol_call` already lives). Recommendation is the former, with
  reconciliation deferred to the Agent 04 #3 file split. (§Fix Design)

- **Whether to also collapse the near-variant alloc sites.** 45 of 124 sites
  are byte-identical; the other 79 differ in alignment argument, destination
  slot, or failure tail. Collapsing only the 45 is low-risk and captures ~55%
  of the value; collapsing all 124 requires a per-site equivalence argument.
  Recommendation: all 124, but phased by file with a gate run between.

## Summary

The engineering risk is not in the helper — it is in the **adoption sweep**
across ~230 sites in 30 files, and in the fact that two incompatible dialects
(`CodeBuilder` methods vs. free functions threading `&mut Vec<_>`) both need
serving. The artifact gate makes this tractable: a pure refactor must produce a
zero-byte artifact diff, so correctness is machine-checkable at every phase
rather than argued. The one thing that must not be swept along silently is the
size-overflow error-code hazard, which a mechanical unification would resolve
one way or the other without anyone deciding to; it is carved out into an
explicit helper parameter and a follow-up bug. Untouched: all emitted bytes,
all relocation ordering, all label names, and every golden.
