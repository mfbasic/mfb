# bug-323: helper-body 4-tuple and the 5-param emitter preamble are spelled out longhand repo-wide (no type alias, no context struct)

Last updated: 2026-07-19
Effort: large (3h–1d)
Severity: LOW
Class: Other (cleanup)

Status: Fixed (2026-07-19). Phase 2 and Phase 3 both complete.

Phase 2: `HelperBody`/`HelperResult`/`AppHookBody` applied at all 115 + 52 sites;
`clippy::type_complexity` 119 -> 6, exactly the out-of-scope set this document
names.

Phase 3: `EmitCtx` declared and adopted at **all 55** preamble carriers across
14 files (the document's count of 43 was low). `too_many_arguments` warnings
31 -> 0; suppressions 54 -> 49, i.e. strictly down as the phase requires — 46
were deleted and only the 40 still needed were re-added. Every file was gated
individually: `scripts/artifact-gate.sh` showed 1,189 goldens / 0 diffs at each
of the 14 steps.

The document deferred Phase 3 as "not neutral by construction" because bundling
two `&mut Vec` forces callers to restructure borrows. That hazard is real but
avoidable: the three shared refs are `&'a` fields, so reading them into locals at
the top of a converted function is independent of the `&mut ctx` borrow. That
keeps `{symbol}` format-string interpolation working (a naive field-access
rewrite breaks it) and leaves only the two streams needing `ctx.`.

Two things only reading the bodies caught, which the gate could not:
`emit_write_string_object`'s parameter named `symbol` is the string DATA symbol
(a relocation's `to`), not the emitter (`from`) — bundling it into `ctx.symbol`
put the wrong value there while emitting identical output. And several helpers
carried a `from`/`symbol` parameter that duplicated `ctx.symbol` exactly; those
are deleted rather than threaded through twice.

Every runtime-helper lowering function in `src/target/shared/code/` returns the same
four-element tuple — frame, instructions, relocations, stack slots — and almost every
one of them takes the same five leading parameters. Neither shape has ever been given
a name. The tuple is re-typed longhand at 115 sites across 22 files and is, by itself,
**113 of the 119 `clippy::type_complexity` warnings in the whole tree**; the parameter
preamble drives 31 `too_many_arguments` warnings plus 51 hand-written suppressions.
The single correct end state a fix produces: the helper-body shape is spelled **once**,
as a named type, and every one of those 115 sites refers to it by name; the emitter
preamble is spelled **once**, as a context struct, and the warning suppressions that
exist only to hide the longhand are deleted rather than kept.

This is not a correctness defect and nothing miscompiles today. It is filed because the
cost is compounding and structural: the tree contains exactly **two** type aliases in
all of `src/` (`src/docs/render.rs:55`, `src/target/shared/code/regalloc/analysis.rs:478`),
which is not a stylistic preference but a systemic absence of the aliasing habit. The
concrete damage is that the repo's largest lint cluster is 95% noise from one unnamed
type, so a genuinely complex signature can never be spotted; and that adding a fifth
element to a helper body (a plausible future — a second relocation plane, a debug-line
table) is a 115-site edit instead of a one-line edit.

References:

- Cleanup review, Agent 22 ("cross-cutting lint clusters") findings #1 and #2.
- Cleanup review, Agent 06 ("fs/io/os/net codegen") finding #23 — independently found
  the same 4-tuple inside its own scope and reached the same conclusion.
- `bugs/bug-300-docs-deadcode-low-cluster.md` — the repo's convention for filing a
  cross-module cleanup cluster as one document.
- `src/docs/spec/architecture/06_native.md` — the native codegen layer this bug is
  confined to.

## Audit addendum (2026-07-19, re-measured at HEAD `4e0b6e04d`)

Re-verified 69 commits after the base this was filed against (`b12213d2`). The
structural claims survived; most line numbers did not, and four claims are wrong.

**Held exactly:** 119 `type_complexity` warnings; 31 `too_many_arguments`
warnings; 9 `#[allow(clippy::type_complexity)]`; 115 4-tuple occurrences across
22 files (**the whole distribution table reproduces row for row**); 52 3-tuple
occurrences across 8 files; only 2 type aliases in `src/`; the 575-line saving.

**Corrections.**

- **The visibility plan does not work for the 3-tuple.** The document says both
  aliases go in `shared/code/mod.rs` as `pub(super)` and reach every file
  through the existing glob import "with no `use` edits." True for the 4-tuple
  (all 22 files carry `use super::*`). **False for the 3-tuple:** 46 of its 52
  sites live outside `crate::target::shared`, where `pub(super)` is not
  nameable, and `linux_{aarch64,x86_64,riscv64}/code.rs` have **no `use
  super::*` at all**. `AppHookBody` needs `pub(crate)` plus explicit `use` edits
  in 6 files. **Resolve this before Phase 2 or 46 sites will not compile.**
- "One occurrence is a call-site expression rather than a signature" — **false**.
  All 115 are type positions. The 115→113 gap is instead: `mod.rs:393`'s
  `#[allow]` suppresses one, and `runtime_helpers_thread.rs:1351` returns a
  **bare** 4-tuple (no `Result` wrapper) so it scores under clippy's threshold.
  Both bare sites take `HelperBody`, **not** `HelperResult` — and because they
  are invisible to clippy, the "119 → 6" acceptance criterion passes whether or
  not they are converted. That is a silent scope leak; check them by hand.
- `#[allow(clippy::too_many_arguments)]` is **54**, not 51 — Phase 1's baseline
  fails on the stale number.
- "This exact block appears 14 times in `os.rs` alone" — **false**. The
  6-parameter list appears **once**; only the 9-line return-type tail recurs 14
  times, across 13 functions with *different* parameter lists.
- The `EmitCtx` carrier count is **43**, not 41 — `audio/alsa.rs` has 5 (not 4)
  and `net/io.rs:34` was omitted entirely.
- "78 functions with ≥8 params" is not reproducible: measured **81** counting
  `self`, **69** excluding it. The document does not record its method.
- Stale line numbers: `io_helpers.rs` allows are `:829/:1082/:1120` (not
  `:807/:1060/:1098`); `lower_builtin_function_wrapper` is `:817-827`;
  `net/poll.rs` is 264 lines with its second signature at `:150`.
- The ⚠ dirty-tree note is stale — `linux_gtk/mod.rs` is clean.

**Must NOT be converted:** `types.rs:134` (`layout_data_objects`) is the same
lint, same file, same arity, but is a data-blob layout tuple, not a codegen
body — it is one of the 6 legitimately out-of-scope warnings. Likewise the
allows at `docs/man/mod.rs:4`, `docs/spec/mod.rs:19`, `cli/resolve.rs:425` are
unrelated and must survive.

**Output-neutrality:** Phase 2 is neutral *by construction* — a Rust type alias
is structurally transparent, producing an identical `TyKind::Tuple` and identical
MIR, with no construction or destructuring site touched. To *prove* it: confirm
`git diff` touches no line containing `Ok((`, `let (`, or `.0`/`.1`/`.2`/`.3`,
and build under **all** backend `cfg`s — a host-only build exercises one of the
five backends the 3-tuple alias touches. Phase 3 (`EmitCtx`) is **not**
provably neutral: bundling two `&mut Vec` into a struct forces callers to
restructure borrows, and only the artifact gate can adjudicate that.

## Current State

All counts below were re-measured against this worktree (`HEAD` = b12213d2), not
carried over from the review notes. Where a review figure did not reproduce, the
measured figure is used and the discrepancy is noted.

### The 4-tuple

Measured by `cargo clippy --all-targets --message-format=short`:

| Metric | Measured |
| --- | --- |
| `clippy::type_complexity` warnings, whole tree | 119 |
| …of which are this one 4-tuple | **113** |
| …everything else (6 unrelated types) | 6 |
| Raw source occurrences of the 4-tuple | **115** across **22** files |
| Source lines consumed purely by re-spelling it | **575** |
| `clippy::too_many_arguments` warnings | 31 |
| Hand-written `#[allow(clippy::too_many_arguments)]` | **51** |
| Hand-written `#[allow(clippy::type_complexity)]` | 9 |
| Type aliases in all of `src/` | **2** |

The gap between 115 occurrences and 113 warnings is exact and accounted for:
`src/target/shared/code/mod.rs:393` carries an `#[allow]` that suppresses one, and one
occurrence is a call-site expression rather than a signature.

Verbatim, as it appears at `src/target/shared/code/os.rs:148-163`:

```rust
pub(super) fn lower_os_helper(
    call: &str,
    symbol: &str,
    build_mode: crate::target::NativeBuildMode,
    module_name: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
```

Nine lines of the fifteen are the return type. This exact block appears 14 times in
`os.rs` alone.

Distribution of the 115 occurrences (all under `src/target/shared/code/`):

| File | Sites | Lines |
| --- | --- | --- |
| `os.rs` | 14 | 70 |
| `fs_helpers_io.rs` | 12 | 60 |
| `io_helpers.rs` | 9 | 45 |
| `fs_helpers_paths.rs` | 9 | 45 |
| `audio/macos.rs` | 9 | — |
| `net/io.rs` | 8 | 40 |
| `tls/openssl.rs` | 7 | — |
| `tls/macos.rs` | 7 | — |
| `audio/alsa.rs` | 7 | — |
| `fs_helpers_atomic.rs` | 6 | 30 |
| `runtime_helpers_thread.rs` | 4 | — |
| `net/mod.rs` | 4 | 20 |
| `crypto_ec/openssl.rs` | 4 | — |
| `crypto_ec/macos.rs` | 4 | — |
| `runtime_helpers.rs` | 3 | — |
| `net/poll.rs` | 2 | 10 |
| `term.rs`, `mod.rs`, `datetime.rs`, `crypto_ec.rs`, `crypto.rs`, `audio/mod.rs` | 1 each | — |

Agent 06's independent finding covered the `fs`/`io`/`os`/`net` subset only. Measured,
that subset is **64 occurrences / 320 removable lines** across 8 files — larger than the
"~35 occurrences / ~280 lines" the reviewer estimated. Agent 06's conclusion holds; its
scope was under-counted.

### The 3-tuple sibling and its hand-rolled adapter

A three-element variant — the same shape minus the stack-slot list — is the app-mode
platform-hook signature. Measured: **52 occurrences across 8 files**
(`target/{linux_aarch64,linux_riscv64,linux_x86_64}/code.rs`,
`target/linux_gtk/{mod,app_io}.rs`, `target/macos_aarch64/app/app_io.rs`,
`target/shared/code/{mod,types}.rs`). The review note's figure of 85 does not
reproduce; 85 appears to come from a regex that also matched the 4-tuple's leading
three elements. A naive prefix count gives 167, which brackets it.

Five of the nine hand-written `type_complexity` allows sit on this sibling, in one
place — the `CodegenPlatform` app-mode hooks at `src/target/shared/code/types.rs:553`,
`:566`, `:577`, `:599`, `:611`, e.g.:

```rust
    #[allow(clippy::type_complexity)]
    fn emit_app_io_flush_helper(
        &self,
        _symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
```

The reviewer's claim that `src/target/shared/code/mod.rs:393` is a hand-rolled 3→4
adapter is **confirmed verbatim** — it is the seam between the two shapes, and it
carries a sixth suppression of its own:

```rust
/// Adapt a not-yet-vreg shaped helper body (3-tuple, e.g. an app-mode platform
/// hook that manages its own frame) to the 4-tuple shape with an empty
/// spill-slot list, so it can share a `match`/`if` with vreg-migrated helpers.
#[allow(clippy::type_complexity)]
fn pad_no_slots(
    body: (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>),
) -> (
    CodeFrame,
    Vec<CodeInstruction>,
    Vec<CodeRelocation>,
    Vec<CodeStackSlot>,
) {
    (body.0, body.1, body.2, Vec::new())
}
```

That doc comment is the clearest statement of the problem in the tree: it has to
explain, in prose, a relationship that two named types would have expressed in their
declarations.

### The parameter preamble

Parameter-name frequency, measured by extracting every `fn` signature in `src/` and
counting top-level parameter declarations:

| Parameter | Measured | Review note |
| --- | --- | --- |
| `platform_imports` | 307 | 347 |
| `instructions` | 306 | 279 |
| `relocations` | 234 | 236 |
| `platform` | 195 | 191 |
| `symbol` | 338 | — |

The magnitudes hold; the individual figures drift by up to 12% from the review note
(that note's method is unrecorded, so the two are not directly reconcilable). The
load-bearing measurement is the co-occurrence: **41 functions take all five of
`symbol` + `platform_imports` + `platform` + `instructions` + `relocations`**, and
**78 functions in `src/` take 8 or more parameters**.

Concentration of the 41:

| File | Functions with the full 5-param preamble |
| --- | --- |
| `io_helpers.rs`, `audio/macos.rs` | 6 each |
| `fs_helpers_io.rs` | 5 |
| `term.rs`, `audio/alsa.rs` | 4 each |
| `os.rs`, `tls/macos.rs`, `tls/mod.rs` | 3 each |
| `stdin_broadcast.rs`, `runtime_helpers_thread.rs` | 2 each |
| `runtime_helpers.rs`, `entry_and_arena.rs`, `net/mod.rs` | 1 each |

The reviewer's cited worst case reproduces exactly — `src/target/shared/code/fs_helpers_io.rs:109`,
`emit_transfer_loop_tail`, 11 parameters, of which the first five are the preamble:

```rust
pub(super) fn emit_transfer_loop_tail(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    ret: &str,
    raw_return: bool,
    cursor: &str,
    remaining: &str,
    loop_label: &str,
    error_label: &str,
) -> Result<(), String> {
```

The remaining six parameters are the function's actual subject. The preamble is 45% of
the signature and carries zero information about what this function does.

### The second parameter cluster: the lowering tables

A distinct group of seven read-only symbol tables always travels together, confirmed at
both cited sites. `src/target/shared/code/function_lowering.rs:557-565`:

```rust
pub(super) fn lower_function(
    function: &NirFunction,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    package_return_types: &HashMap<String, String>,
    platform_imports: &HashMap<String, String>,
    globals: &HashMap<String, GlobalValue>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
```

and, with the same seven in the same order,
`src/target/shared/code/function_lowering.rs:811-821` (`lower_builtin_function_wrapper`,
10 params). Both then copy all seven, field by field, into the `CodeBuilder` literal —
`function_lowering.rs:836-843` is that transcription. This is a struct that was never
declared, being passed as loose fields and reassembled at the destination.

### Where the 6 non-4-tuple `type_complexity` warnings are

Bounding the cluster — these are genuinely distinct types and are **out of scope**:

- `src/binary_repr/writer.rs:39`
- `src/cli/resolve.rs:224`
- `src/manifest/package.rs:345`
- `src/target/shared/code/types.rs:134`
- `src/target/shared/code/mir.rs:585`
- `src/os/linux/squashfs/tests.rs:367`

## Root Cause

There is no mechanism failure here — the cause is that the codegen layer grew by
copy-paste from a single seed helper, and Rust does not force a returned tuple to be
named. `src/target/shared/code/os.rs:148` is representative: the first `os::` helper
was written with an inline tuple, and each subsequent helper was authored by copying a
neighbour. Because `clippy::type_complexity` fires on the *signature* rather than the
*duplication*, the tree accumulated 113 identical warnings that read as 113 separate
problems, and the local remedy each author reached for was `#[allow]` — 9 times for the
type, 51 times for the arity.

The absence of the aliasing habit is what let this run to 115 sites. With two aliases in
the entire `src/` tree, there was no local precedent for an author to copy: the path of
least resistance was always to re-spell the tuple. `src/target/shared/code/mod.rs:393`
`pad_no_slots` is the point where the cost became visible enough to warrant a comment,
and even there the response was an adapter function plus a suppression rather than two
names.

The `CodegenPlatform` app-mode hooks (`types.rs:553-611`) are immune to the 4-tuple
count only because they predate the vreg migration and return the 3-tuple; they carry
the identical problem in its own five-site cluster.

## Goal

- `clippy::type_complexity` reports **6** warnings tree-wide (down from 119), and the 6
  are the unrelated types enumerated above. No new `#[allow(clippy::type_complexity)]`
  is added; the 6 that exist only to hide this shape
  (`mod.rs:393`, `types.rs:553/566/577/599/611`) are **deleted**, not relocated.
- The helper-body shape is declared exactly once and referenced by name at all 115 sites.
- The 3-tuple sibling is declared exactly once and referenced by name at its 52 sites;
  `pad_no_slots` becomes a one-line conversion between two named types.
- `EmitCtx<'a>` exists and is threaded through the 41 functions carrying the full
  preamble; `LoweringTables<'a>` exists and replaces the seven loose tables at
  `function_lowering.rs:557` and `:811`.
- The number of `#[allow(clippy::too_many_arguments)]` in `src/` strictly decreases, and
  every one that remains is justified by a comment naming the parameters that are
  genuinely irreducible.
- **`scripts/artifact-gate.sh` and `scripts/test-accept.sh` pass with zero diff.**

### Non-goals (must NOT change)

- **Generated output must be byte-identical.** This is the governing constraint of the
  whole bug. Every phase is gated on `scripts/artifact-gate.sh` (execution-free,
  ~5 min) and `scripts/test-accept.sh`. Not one emitted instruction, relocation, stack
  slot, frame size, symbol name, or golden file may shift. If a phase produces any
  artifact delta, that phase is **wrong** and must be reverted, not re-baselined.
- No change to the *values* any helper computes, the order in which instructions are
  pushed, or the order in which relocations are appended. Reordering struct fields is
  free; reordering `instructions.extend([...])` is not.
- No change to `CodeFrame`, `CodeInstruction`, `CodeRelocation`, `CodeStackSlot`,
  `CodeFunction`, or `NativeCodePlan` — the alias names the *tuple of them*, it does not
  restructure them.
- No change to the `.mfp` wire format, the `-mir`/`-nplan`/`-nobj` dumps, or any golden.
- No change to the `CodegenPlatform` trait's *set* of methods or its dispatch semantics;
  only the spelling of five return types.
- **Forbidden wrong fix:** promoting `type_complexity`/`too_many_arguments` to
  `allow` at the crate root, or adding file-level `#![allow]`s. That would zero the
  warning count while leaving all 115 sites and every future maintenance edit intact —
  it inverts the goal. (Contrast Agent 22 #3, where a file-level allow *is* the right
  answer, because those 49 `excessive_precision` constants are deliberate and there is
  nothing to factor.)
- Do not fold the 3-tuple into the 4-tuple by giving app-mode hooks an empty slot
  vector. `pad_no_slots`'s doc comment records that these hooks manage their own frame;
  collapsing the distinction is a semantic change disguised as a cleanup.

## Blast Radius

Found by direct search, not from memory. Every site is inside `src/target/`, which
bounds this bug to the native codegen layer.

**Fixed by this bug — the 4-tuple (115 sites, 22 files):** all files in the
distribution table above. Mechanical: each is a signature or a call-site type
annotation.

**Fixed by this bug — the 3-tuple (52 sites, 8 files):**
`src/target/shared/code/types.rs` (the 5 trait hooks), `src/target/shared/code/mod.rs`
(`pad_no_slots` + call sites), `src/target/linux_aarch64/code.rs`,
`src/target/linux_x86_64/code.rs`, `src/target/linux_riscv64/code.rs`,
`src/target/linux_gtk/mod.rs`, `src/target/linux_gtk/app_io.rs`,
`src/target/macos_aarch64/app/app_io.rs` (the impls).

⚠ `src/target/linux_gtk/mod.rs` is **modified in the working tree** at the time of
filing (plan-56-A phase 1, commit b12213d2 flavored the GTK app-mode import surface).
Re-measure its site count before editing it.

**Fixed by this bug — the preamble (41 functions):** concentrated per the table above.

**Fixed by this bug — the lowering tables (2 functions):**
`function_lowering.rs:557` (`lower_function`), `:811`
(`lower_builtin_function_wrapper`), plus the `CodeBuilder` literal transcription at
`:836-843` in each.

**Latent, same hazard, out of scope:**

- The 6 unrelated `type_complexity` sites listed above — each is a genuinely distinct
  type with one occurrence; an alias for a single use is churn, not cleanup.
- The 78 functions with ≥8 parameters that do *not* carry the preamble (78 − 41 = 37).
  Their width is intrinsic to what they do; `EmitCtx` would not shrink them, and
  inventing a struct per call shape would be worse than the status quo. Out of scope
  because there is no shared shape to name.
- `src/target/shared/code/entry_and_arena.rs` and `link_thunk.rs` hold the widest
  individual signatures in the tree. They carry the preamble (1 function each) and are
  in scope for that, but their residual width is out of scope — see Agent 05 #1 and
  Agent 07 #1, which propose splitting those modules for independent reasons. **Do not
  land this bug's changes and a module split in the same commit**; the artifact gate
  cannot attribute a diff between two entangled refactors.

**Unaffected:**

- Everything outside `src/target/`. No front-end, IR, NIR, linker, or CLI signature
  carries either shape.
- `src/docs/render.rs:55` and `src/target/shared/code/regalloc/analysis.rs:478` — the
  two existing aliases are correct and unrelated; they are cited only as evidence of
  how rare the habit is.

## Fix Design

The correctness risk in this bug is close to zero and the *review* risk is close to
total: this is ~700 lines of diff across 30 files in which a single transposed tuple
element would produce a wrong binary that still compiles. Every design choice below
optimizes for reviewability and for the artifact gate's ability to catch a mistake,
not for elegance.

### The helper body — two options

**(a) A struct with named fields.**

```rust
pub(super) struct HelperBody {
    pub frame: CodeFrame,
    pub instructions: Vec<CodeInstruction>,
    pub relocations: Vec<CodeRelocation>,
    pub stack_slots: Vec<CodeStackSlot>,
}
pub(super) type HelperResult = Result<HelperBody, String>;
```

Strictly better as an end state: `body.instructions` cannot be confused with
`body.relocations`, whereas `body.1` and `body.2` are one keystroke apart and both
`Vec`. It also makes a future fifth element additive.

Cost: it churns **every destructuring call site**, not just the 115 signatures. Every
`let (frame, instructions, relocations, slots) = lower_x(...)?;` and every
`Ok((frame, insts, relocs, slots))` becomes a struct literal or a field pattern — and
each of those is a hand-edit where a mis-transcription silently swaps two `Vec`s of the
same type. The 4-tuple's positional construction sites are where the byte-identical
guarantee is most fragile, and option (a) rewrites all of them.

**(b) A bare tuple alias.**

```rust
pub(super) type HelperBody = (
    CodeFrame,
    Vec<CodeInstruction>,
    Vec<CodeRelocation>,
    Vec<CodeStackSlot>,
);
pub(super) type HelperResult = Result<HelperBody, String>;
```

Kills all 113 warnings and removes all 575 lines. Every construction and destructuring
site is **untouched** — a tuple alias is transparent, so `Ok((frame, insts, relocs,
slots))` and `let (a, b, c, d) = ...` keep compiling verbatim. The diff is confined to
signatures, and the compiler proves each one: a wrong alias cannot type-check.

**RECOMMENDATION: (b), the bare alias.**

The reason is the Non-goal. Option (b) is *provably* byte-identical-preserving — it
changes only how a type is spelled, and `rustc` rejects any error. Option (a) is
byte-identical-preserving only if ~230 hand-edited construction and destructuring sites
are each transcribed correctly, and the compiler will accept a swap of `instructions`
and `relocations` at any of them because both are `Vec`-typed and the struct-literal
field order is unchecked. Trading a mechanically-verified refactor for a
manually-verified one, in exchange for field names, is the wrong trade at this size.
Correctness over ergonomics (and over performance — the same principle).

This is not a permanent rejection of (a). The alias is a strict prerequisite for it: once
`HelperBody` exists as a name, converting it from a tuple to a struct is a *localized*
follow-up that can be staged one module at a time behind the same artifact gate, with
each step small enough to review. Land (b) now; file (a) as a follow-up if the field
names prove worth the churn. Do not attempt (a) first.

The 3-tuple gets the same treatment: `type AppHookBody = (CodeFrame,
Vec<CodeInstruction>, Vec<CodeRelocation>);` — named for what it is (the app-mode
platform hook shape), not `HelperBody3`. `pad_no_slots` then reads
`fn pad_no_slots(body: AppHookBody) -> HelperBody`, its `#[allow]` is deleted, and its
three-line doc comment can shrink to one, because the signature now says what the prose
was saying.

**Placement:** both aliases in `src/target/shared/code/mod.rs`, beside `pad_no_slots`,
which is the one place that already knows about both. `pub(super)` matches the
visibility of the functions that use them. Note that `mod.rs` glob-exports to its
children (Agent 04 #20), so the aliases will be in scope in every affected file with no
`use` edits — convenient here, but do not add new globs to exploit it.

### The parameter preamble — `EmitCtx<'a>`

```rust
pub(super) struct EmitCtx<'a> {
    pub symbol: &'a str,
    pub platform_imports: &'a HashMap<String, String>,
    pub platform: &'a dyn CodegenPlatform,
    pub instructions: &'a mut Vec<CodeInstruction>,
    pub relocations: &'a mut Vec<CodeRelocation>,
}
```

Unlike the alias, this one *does* change call sites, so it cannot be done tree-wide in
one pass. The two `&mut` fields are the constraint: bundling them into one struct means
a caller can no longer hold an independent borrow of `instructions` while passing
`relocations`. Some existing call sites will fight this. That fight is the actual work
of the phase, and it is why the preamble is staged after the alias and module by module.

### The lowering tables — `LoweringTables<'a>`

```rust
pub(super) struct LoweringTables<'a> {
    pub function_symbols: &'a HashMap<String, String>,
    pub functions: &'a HashMap<String, &'a NirFunction>,
    pub package_return_types: &'a HashMap<String, String>,
    pub platform_imports: &'a HashMap<String, String>,
    pub globals: &'a HashMap<String, GlobalValue>,
    pub string_symbols: &'a HashMap<String, String>,
    pub type_model: TypeModel,
}
```

Deliberately separate from `EmitCtx`. They overlap in exactly one field
(`platform_imports`) and have opposite lifecycles: `EmitCtx` is per-emitter-call and
holds two `&mut` sinks; `LoweringTables` is per-function-lowering and is entirely
read-only apart from `type_model` (which is moved, not borrowed — preserve that; it is
cloned by the caller today). Merging them would give the read-only tables a mutable
borrow they do not need and would drag `EmitCtx` into `lower_function`'s signature for
no reason.

The payoff at `function_lowering.rs:836-843` is that the seven-field transcription into
the `CodeBuilder` literal becomes a spread of one value.

### Land-first module

**`src/target/shared/code/net/poll.rs`** (255 lines) is the pilot for the preamble work.
It is by an order of magnitude the smallest file carrying the shape: 2 four-tuple sites
(`:19-21` and `:151-153` are its two helper signatures, each taking
`symbol`/`platform_imports`/`platform`), zero `too_many_arguments` allows, one concern,
no cross-backend `cfg`, and no app-mode interaction. If `EmitCtx` cannot be threaded
through `poll.rs` without an artifact diff, the design is wrong and that is discovered
in 20 minutes rather than after 40 functions.

Second module: **`src/target/shared/code/io_helpers.rs`** — 6 preamble functions and
3 of the 51 allows (`:807`, `:1060`, `:1098`), enough to prove the pattern retires
suppressions rather than merely relocating them.

Explicitly **not** first: `os.rs` (14 sites) and `fs_helpers_io.rs` (12 sites, and the
11-param worst case). They are the biggest prize and the worst pilot.

### Rejected alternatives

- **Crate-level or file-level `allow` for either lint.** Forbidden in Non-goals. Zeroes
  the count, fixes nothing.
- **Option (a) first.** Rejected above — right end state, wrong first move.
- **One mega-commit.** The artifact gate is pass/fail over the whole tree; a diff in a
  1-commit, 30-file change is unbisectable. Every phase below must be independently
  gated and independently committable.
- **Landing this alongside the module splits** proposed by Agent 04 #3, Agent 05 #1,
  Agent 07 #1, or Agent 09 #13. Same reason: entangled refactors make an artifact diff
  unattributable.

## Phases

### Phase 1 — baseline + audit (no source change)

- [ ] Record the pre-change baseline: `cargo clippy --all-targets --message-format=short
      2>&1 | grep -c 'very complex type'` → expect **119**; `grep -c 'too many
      arguments'` → expect **31**; `grep -ro 'allow(clippy::too_many_arguments)' src/ |
      wc -l` → expect **51**.
- [ ] Capture a clean `scripts/artifact-gate.sh` run as the reference artifact set. This
      is the oracle for every subsequent phase.
- [ ] Re-measure `src/target/linux_gtk/mod.rs` (dirty in the working tree; see Blast
      Radius) and either commit or stash the plan-56-A change first, so the gate has a
      clean base.

Acceptance: baseline counts recorded in this file; artifact gate green on an unmodified
tree.
Commit: —

### Phase 2 — the `HelperBody` / `AppHookBody` aliases

- [ ] Declare both aliases plus `HelperResult` in `src/target/shared/code/mod.rs`,
      beside `pad_no_slots`.
- [ ] Replace all 115 four-tuple spellings across the 22 files. Signatures only — do not
      touch any construction or destructuring expression.
- [ ] Replace all 52 three-tuple spellings across the 8 files, including the 5
      `CodegenPlatform` hooks at `types.rs:553-611`.
- [ ] Rewrite `pad_no_slots` as `fn pad_no_slots(body: AppHookBody) -> HelperBody` and
      **delete** its `#[allow(clippy::type_complexity)]` (`mod.rs:393`) and the 5 at
      `types.rs`.
- [ ] Confirm `type_complexity` is down to exactly 6, and that the 6 are the enumerated
      unrelated sites.

Acceptance: 6 `type_complexity` warnings; 6 `#[allow(clippy::type_complexity)]` deleted;
**artifact gate byte-identical**; acceptance suite green.
Commit: —

### Phase 3 — `EmitCtx`, module by module

- [ ] Declare `EmitCtx<'a>`.
- [ ] Convert `net/poll.rs` (pilot). Gate. **If any artifact diff appears, stop and
      revisit the design** — do not proceed to the next module.
- [ ] Convert `io_helpers.rs`. Gate. Confirm at least 2 of its 3
      `too_many_arguments` allows are deleted, not moved.
- [ ] Convert the remaining preamble carriers in descending size:
      `audio/macos.rs`, `fs_helpers_io.rs`, `term.rs`, `audio/alsa.rs`, `os.rs`,
      `tls/macos.rs`, `tls/mod.rs`, `stdin_broadcast.rs`, `runtime_helpers_thread.rs`,
      `runtime_helpers.rs`, `entry_and_arena.rs`, `net/mod.rs`. **One commit per file**,
      each gated.
- [ ] For every `#[allow(clippy::too_many_arguments)]` that survives, add a one-line
      comment naming the irreducible parameters.

Acceptance: per-file artifact gate green at every step; suppression count strictly down;
no remaining unjustified allow in a converted file.
Commit: —

### Phase 4 — `LoweringTables`

- [ ] Declare `LoweringTables<'a>`; convert `function_lowering.rs:557` and `:811`.
- [ ] Collapse the seven-field transcription into the `CodeBuilder` literal at
      `:836-843` in both.
- [ ] Update the (few) callers of both functions.

Acceptance: artifact gate byte-identical; both signatures at or below 4 parameters.
Commit: —

### Phase 5 — full validation

- [ ] `scripts/artifact-gate.sh` — zero diff against the Phase 1 reference.
- [ ] `scripts/test-accept.sh` — full acceptance suite, zero golden churn.
- [ ] `cargo fmt` (remember the second pass in `repository/`, which is not a workspace
      member) and `cargo clippy --all-targets`.
- [ ] Re-run the Phase 1 count commands; record the final figures in this file.
- [ ] Build on every supported target combination that the gate covers, confirming the
      alias introduces no `cfg`-dependent breakage — `types.rs` app-mode hooks are
      per-platform and only two backends implement them non-trivially.

Acceptance: full suite green; **zero** artifact bytes changed; final counts recorded.
Commit: —

## Validation Plan

- Regression test(s): **none added, by design.** This bug's correctness claim is
  "nothing changed", which no unit test can express better than the byte-identical
  artifact gate. Adding a test here would assert something the change does not affect.
- Runtime proof: `scripts/artifact-gate.sh` diffing every emitted artifact against the
  Phase 1 reference — the direct, complete proof for a refactor whose entire contract is
  output invariance. `scripts/test-accept.sh` confirms no golden moved.
- Lint proof: `type_complexity` 119 → 6; `too_many_arguments` allows 51 → strictly
  fewer, each survivor justified in a comment.
- Doc sync: none expected. `src/docs/spec/architecture/06_native.md` describes the
  codegen layer's structure, not its Rust signatures, and no spec anchor cites any of
  the affected lines. Confirm with a spec-anchor sweep before closing; Agent 02 #10 and
  Agent 04 #17 report that a handful of anchors are raw line numbers, so a large diff in
  `src/target/shared/code/` may shift one even though no spec *claim* changes.
- Full suite: `scripts/artifact-gate.sh` + `scripts/test-accept.sh` + `cargo test`.

## Open Decisions

- **Helper-body representation** — **recommended: (b) bare tuple alias**, on
  byte-identical-preservation grounds (§Fix Design). Alternative: (a) `HelperBody`
  struct with named fields, which is the better end state but converts a
  compiler-verified refactor into a hand-verified one across ~230 construction sites.
  Recommendation is to land (b) and file (a) as a stageable follow-up, not to skip (a)
  permanently.
- **`EmitCtx` scope** — recommended: the 41 functions carrying all five preamble
  parameters. Alternative: extend to functions carrying 3 or 4 of the 5 (e.g.
  `net/poll.rs`, which takes `symbol`/`platform_imports`/`platform` but creates its own
  instruction vector). Defer until the pilot shows whether a partial `EmitCtx` reads
  better than three loose parameters.
- **3-tuple alias name** — recommended `AppHookBody`. Alternative `PlatformHookBody`.
  Whichever is chosen must not be `HelperBody3`; the two shapes differ in meaning, not
  in arity, and `pad_no_slots`'s doc comment says so.

## Summary

The engineering risk is **entirely in the diff, not the design**. The design is two type
aliases and two structs; the risk is that a ~700-line mechanical edit across 30 codegen
files silently transposes two same-typed `Vec`s and produces a wrong binary that
compiles cleanly. That risk is why the recommendation is the bare tuple alias over the
named-field struct — the alias is verified by `rustc`, the struct is verified by the
author's eyes — and why every phase is one file, one commit, one artifact-gate run.

Untouched: all emitted output (the governing constraint), every code/frame/relocation
type, the `.mfp` format, all goldens, the `CodegenPlatform` method set, and the 37
wide-but-shapeless functions and 6 unrelated complex types that share no pattern with
this cluster.

The measured payoff: 119 → 6 `type_complexity` warnings, 575 lines of re-spelled tuple
deleted, 6 `type_complexity` suppressions and a majority of 51 `too_many_arguments`
suppressions retired, and the repo's largest lint cluster reduced to the handful of
signatures that are actually complex — so that the next one that appears is visible.
