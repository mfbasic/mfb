# Codegen Extraction — `src/codegen` scaffolding + `collections::get` exemplar Plan

Last updated: 2026-08-10
Effort: x-large (1d–3d)

<!-- NOTE: the write-plan split rule says split an x-large plan into medium/large
     sub-plans. The requester explicitly asked for a SINGLE file regardless of
     size ("write a single file plan for this. I dont care the size."), so this
     stays one file. The phases are still ordered and independently-landable, so
     it can be executed straight through or lifted into sub-plans later. -->

This plan stands up the machinery to move MFBASIC's target-agnostic ("shared")
codegen out of `src/target` and into a new `src/codegen` layer, with a builtin's
lowering owned by its registry `Implementation`, and proves the whole pattern by
**fully migrating one function — `collections::get` — end to end, byte-identical
on every target.** It delivers: the `src/codegen` layer; the builtin registry
relocated into it; an `Implementation::Native` variant carrying a lowering fn
pointer; a dual-path dispatch seam (the existing `src/target` ladder by default,
the `Implementation` lowering when a function defines one); and `collections::get`
lowered through that new path, its old `lower_collection_get` deleted, its
descriptor/docs/lowering co-located in `src/codegen/builtins/collections/func_get.rs`.

The single behavioral outcome a correct implementation produces: **the compiler
emits byte-for-byte identical native code before and after, on all five
byte-identity targets, for a program that exercises `collections::get` — while
`get`'s lowering is now reached through `BuiltinFunction.implementation` and no
`lower_collection_get` symbol remains in `src/target`.** Every other builtin
continues to lower through the untouched `src/target` ladder.

References:

- `.ai/testing-gates.md` — artifact-gate / byte-identity harness (the acceptance gate for this plan).
- `.ai/codegen-invariants.md`, `.ai/arch-abi.md` — the `abi::` seam that makes shared lowering target-generic.
- `src/target/shared/code/mod.rs:201` (`CodeBuilder<'a>`), `:496` (`ValueResult`); `src/target/shared/nir/mod.rs:253` (`NirValue`); `src/target/shared/code/operand.rs:132` (`Operand`).
- `src/builtins/descriptor.rs` — the registry types (`Implementation:152`, `BuiltinFunction:183`, `BuiltinModule:325`, `REGISTRY:652`).
- `scripts/artifact-gate.sh`, `scripts/regen-ncodesum.sh`; `tests/byte-identity/collections/` (the `get`-covering fixture).

## Prerequisites

These are a precondition on the whole plan, not scope to negotiate.

| Must be true | Command | Status |
|---|---|---|
| Working tree clean on the target branch | `git status --porcelain` → empty | MET (verified 2026-08-10, 0 dirty) |
| Collections byte-identity gate is GREEN at HEAD | `bash scripts/artifact-gate.sh target/release/mfb collections` → `0 diff(s)` | MET (verified 2026-08-10, 0 diffs) |
| Full suite green at HEAD | `cargo test --bin mfb` → `0 failed` | MET (verified 2026-08-10, 3835 passed) |

Everything below is written against the world where these hold. If the collections
gate is RED at HEAD, STOP and resolve it first — a pre-existing diff would make
this plan's byte-identity acceptance unreadable.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop.

## 1. Goal

- A `program.mfb` that calls `collections::get` (list and map overloads) builds to
  **byte-identical `.ncode` on all five targets** before and after this plan
  (`linux-x86_64`, `linux-aarch64`, `linux-riscv64`, `macos-aarch64`,
  `windows-x86_64`), verified by `artifact-gate.sh collections`.
- `get` is lowered by a fn pointer held in `BuiltinFunction.implementation`
  (`Implementation::Native(..)`), reached through **one** dual-path dispatch seam,
  not by the `if native == Some("get")` / `match … Some("get")` arms.
- `grep -rn 'fn lower_collection_get' src/target` → **no matches** (the old symbol
  is gone), and `grep -rn 'lower_collection_get' src` finds it only in
  `src/codegen/builtins/collections/func_get.rs`.
- `src/codegen/` exists and owns: the registry types (`registry.rs`), the
  `collections` package module (`builtins/collections/`), and `get`'s
  self-contained file (`builtins/collections/func_get.rs`: its `BuiltinFunction`
  entry, doc consts, `Implementation`, and lowering fn).
- Full `cargo test --bin mfb` stays green; a recompiled sample app that calls
  `collections::get` runs and prints the expected value.

### Non-goals (explicit constraints)

- **No behavior change, anywhere.** This is a provably-neutral refactor: same
  emitted bytes, same diagnostics, same runtime results. Byte-identity is the gate.
- **`CodeBuilder`, `NirValue`, `ValueResult`, `Operand` do NOT physically move** in
  this plan (see §3, Open Decisions). They stay in `src/target` and are referenced
  from `src/codegen`. Relocating the 126,330-line / 40-`impl`-file `CodeBuilder`
  body is deferred until enough functions have migrated to reveal its minimal
  surface — that is a later plan.
- **No other builtin migrates.** Only `collections::get`. Every other member —
  including every other `collections::*` — keeps lowering through the `src/target`
  ladder, unchanged.
- **No change to the `abi::` seam** or any per-arch/os backend. The lowering that
  moves speaks only `abi`; nothing that reaches a concrete register/syscall moves.
- No change to any public CLI surface, man pages, or `.mfb` source companion.

## 2. Current State

Shared codegen lives under `src/target/shared/code/` (126,330 lines across 125
files; `CodeBuilder`'s behavior alone is spread over 40 `impl CodeBuilder` files).
It is *misfiled*: the `lower_*` functions have no per-arch branches — they emit
abstract instructions through the `abi::` module, which resolves registers and
calling conventions per target. The proof they are target-generic (not
target-independent) is that the byte-identity goldens **differ per target** from
the *same* source (`collections_codegen_cover_rt.linux-x86_64.ncodesum` ≠
`.macos-aarch64.ncodesum`).

A builtin call reaches its lowering by a **stringly-typed dispatch**, with no link
from the descriptor to the code:

1. `BuiltinFunction { name: "collections.get", … }` in `src/builtins/descriptor.rs`
   drives type-checking, arity, docs, errors — never codegen.
2. At lowering, `crate::builtins::native_builtin_target(target)`
   (`src/builtins/mod.rs:208`) maps the call target to the bare member `"get"`.
3. `src/target/shared/code/builder_values.rs` dispatches on that string at **two**
   sites: an `if native == Some("get") && args.len() == 2` arm (`:727`, the
   inline/raw-supported path) and a `match … { Some("get") => … }` arm (`:1795`,
   the normal call-lowering path). Both call
   `self.lower_collection_get(args)` (`builder_collection_queries.rs:34`,
   `pub(super) fn lower_collection_get(&mut self, args: &[NirValue]) -> Result<ValueResult, String>`).
4. `lower_collection_get` inspects `args[0]`'s type and branches to
   `lower_list_get` / `lower_map_get` (same file).

The registry (`BuiltinModule`, `BuiltinFunction`, `Implementation`,
`BuiltinRegistry`, `REGISTRY`, `Parameter`, `BuiltinOverload`, `DefaultResolver`,
`BuiltinResolver`, …) all lives in `src/builtins/descriptor.rs`. `Implementation`
is today an anemic proxy: `enum Implementation { Same, Rewrite(&'static str), Custom }`
— it describes name→symbol mapping, not the actual implementation.

Precedent to mirror: the existing byte-identity migration idiom (plan-88's
`raise_error` swap, `ba117862e`'s golden regen) — a code-motion change reached via
a new indirection is byte-neutral, verified by `artifact-gate.sh`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Lines in `src/target/shared/code` | 126,330 | `find src/target/shared/code -name '*.rs' -exec cat {} + | wc -l` |
| Files in `src/target/shared/code` | 125 | `find src/target/shared/code -name '*.rs' | wc -l` |
| Files with `impl CodeBuilder` | 40 | `grep -rl 'impl CodeBuilder' src/target/shared/code | wc -l` |
| `CodeBuilder` fields | ~56 | `awk` over `mod.rs:201..struct-end` (approx) |
| `lower_collection_get` call sites | 2 | `grep -rn 'self.lower_collection_get(' src/target/shared/code` → `builder_values.rs:727,1795` |
| Files importing the descriptor module | 31 | `grep -rl 'super::descriptor\|builtins::descriptor\|descriptor::' src | wc -l` |
| Files consuming `builtins::collections` API | 10 | `grep -rl 'builtins::collections' src | grep -v collections.rs | wc -l` |
| Collections byte-identity targets | 5 | `ls tests/byte-identity/collections/golden/*.ncodesum | wc -l` |

### Verified properties

- **`lower_collection_get` has exactly two callers**, both in `builder_values.rs`
  (`:727`, `:1795`), both keyed on `native_builtin_target(target) == Some("get")`.
  Verified by reading both sites. The dual-path seam must cover BOTH; removing the
  old symbol without both fails to compile (caught by the compiler).
- **`ValueResult` couples to `Operand`** (`{ type_: String, location: Operand, text: String }`,
  `mod.rs:496`; `Operand` at `operand.rs:132`), and **`NirValue` is the whole NIR
  enum** (`nir/mod.rs:253`) — verified by reading. This is why moving the value
  types is *not* cheap and is a non-goal here: the fn-pointer type merely *names*
  them; they stay in `src/target`.
- **`CodeBuilder<'a>` carries a lifetime** — the `Native` fn-pointer type needs a
  higher-ranked bound (`for<'a> fn(&mut CodeBuilder<'a>, …)`). Verified from the
  struct decl at `mod.rs:201`. Free functions satisfy HRTB, so a `const`-stored
  pointer is expressible.
- **`get`'s lowering speaks only `abi`** (no raw register/syscall) — verified by
  reading `lower_collection_get` / `lower_list_get` / `lower_map_get`: they use
  `self.emit(abi::…)`, `self.allocate_stack_object`, `self.lower_value`, labels,
  and the error-emission helpers. So `get` is a valid `src/codegen` candidate; the
  helpers it calls become the measured visibility surface (see §Detailed Design).

## 3. Design Overview

Four independent pieces, layered:

1. **`Implementation::Native`** — grow the registry enum to carry the lowering:
   `Native(NativeLower)` where
   `type NativeLower = for<'a> fn(&mut CodeBuilder<'a>, &[NirValue]) -> Result<ValueResult, String>`.
   This is the ONE place the registry gains a dependency on the codegen types.
2. **Dual-path dispatch** — one seam, consulted at both `builder_values.rs` sites
   before the existing ladder: if `REGISTRY.function(target)` has
   `Implementation::Native(f)`, `return f(self, args)`; else fall through to the
   ladder unchanged. This is what lets migration proceed one function at a time
   with the ladder as a permanent fallback.
3. **`src/codegen` layer** — new module owning `registry.rs` (moved from
   `src/builtins/descriptor.rs`) and `builtins/collections/` (moved from
   `src/builtins/collections.rs`). `src/builtins` and `src/target` both depend on
   `src/codegen::registry`.
4. **`get` migration** — `lower_collection_get` becomes a free fn `lower_get` in
   `src/codegen/builtins/collections/func_get.rs`; `get`'s `BuiltinFunction` points
   `implementation` at it; the two ladder arms and the old method are deleted.

**Byte-identity IS this plan's correctness gate.** Every phase is provably-neutral
code motion / indirection — the emitted `.ncode` must not change on any target. So
each phase's acceptance is `artifact-gate.sh collections` byte-identical on all 5
targets **plus** full `cargo test --bin mfb` green. A diff is NOT a
premise-falsification — it is a bug the phase introduced: objdump/`.ncode`-dump one
fixture, root-cause it, fix it, and the gate passes. (Reaching `lower_get` via a fn
pointer vs a match arm emits identical instructions — this is the same neutrality
the `raise_error` swap relied on.)

**Where risk concentrates.** Design *uncertainty* is entirely in piece 1+2: "can a
`const` `BuiltinFunction` hold a lowering fn pointer (HRTB over `CodeBuilder<'a>`),
and can dispatch prefer it, byte-identically?" That is cheap to falsify and is
scheduled **first** (Phase 1), wired to the *existing* `lower_collection_get` with
zero code moved. *Blast radius* is in the relocations (31-file registry re-import,
10-file collections re-import) and the old-symbol deletion — scheduled last, behind
the proven mechanism and the byte gate.

**The temporary layering wart (accepted, and named).** Because `CodeBuilder` stays
in `src/target` this plan, `src/codegen::registry` will `use
crate::target::shared::code::{CodeBuilder, ValueResult}` and
`crate::target::shared::nir::NirValue` for the `Native` variant, and `src/target`
will keep reading `crate::codegen::registry::REGISTRY`. That is an intra-crate
`codegen ↔ target` cycle. Rust permits it; it is the deliberate intermediate state.
It resolves in a later plan when `CodeBuilder` itself relocates into `src/codegen`
(once its consumers are all in codegen and its surface is known). Do not try to
break the cycle now by abstracting `CodeBuilder` behind a trait — its ~56-field /
40-`impl`-file surface is too large to abstract, and the byte-identity requirement
means the concrete type is what must be called.

### Rejected alternatives

- **Move `CodeBuilder`/`NirValue`/`ValueResult` first (the literal Step 1).**
  Rejected for THIS plan: 126,330 lines, 40 `impl` files, and a
  `ValueResult→Operand` / `NirValue=whole-NIR` cascade — a multi-day move with no
  verification signal but "it compiles," done *before* the mechanism is proven.
  Inverts uncertainty-first. Deferred to a dedicated later plan; see Open Decisions.
- **A `Lowerer` trait abstracting `CodeBuilder`.** Rejected: the surface is too
  large to abstract, and a trait-object indirection risks perturbing codegen
  (byte-identity). The fn pointer over the concrete type is the neutral choice.
- **Keep the registry in `src/builtins`, only add `Native`.** Rejected: the
  requester wants `src/codegen` to own the registry as part of "everything in
  place." Moving it now (pure data + the one `Native` edge) is mechanical and sets
  the final layer direction.
- **Migrate `resolve_get` (type resolution) too, in this plan.** Deferred; see Open
  Decisions. This plan migrates *lowering* only; `get`'s type rule stays in the
  shared `CollectionsResolver` for now.

## 4. Detailed Design

### 4.1 `Implementation::Native` and the fn-pointer type

In the moved registry (`src/codegen/registry.rs`):

```rust
pub(crate) type NativeLower =
    for<'a> fn(&mut CodeBuilder<'a>, &[NirValue]) -> Result<ValueResult, String>;

pub(crate) enum Implementation {
    Same,
    Rewrite(&'static str),
    Custom,
    /// The function is lowered by this target-generic fn (reached via the codegen
    /// dual-path seam). Present only for migrated functions.
    Native(NativeLower),
}
```

- `Implementation` currently derives `Clone, Copy, PartialEq, Eq, Debug`. Fn
  pointers are `Copy` and `PartialEq` (by address) but `Eq`/`Debug` are awkward:
  keep `Clone, Copy`; **replace `derive(Debug)` with a hand-written `Debug`** that
  prints `Native(<fn>)`; **drop `PartialEq/Eq`** unless a consumer needs them
  (audit: `grep -rn 'Implementation::' src | grep '=='`). If any equality consumer
  exists, give `Native` a `PartialEq` that compares by `fn as usize`.
- A tiny accessor keeps the seam clean:
  `impl BuiltinFunction { pub(crate) fn native_lower(&self) -> Option<NativeLower> { match self.implementation { Implementation::Native(f) => Some(f), _ => None } } }`.
- `DefaultResolver::implementation_name` and any `match` on `Implementation` gain a
  `Native(_) => None` arm (it has no rewrite symbol). Audit those arms.

### 4.2 Dual-path dispatch seam

Add one helper on `CodeBuilder` (in `builder_values.rs`, next to the ladder):

```rust
fn try_native_lower(&mut self, target: &str, args: &[NirValue])
    -> Option<Result<ValueResult, String>>
{
    let f = crate::codegen::registry::REGISTRY.function(target)?.1.native_lower()?;
    Some(f(self, args))
}
```

`target` at both call sites is the fully-qualified call name (e.g.
`"collections.get"`) that `native_builtin_target` also consumes — confirm the exact
string form at each site and key `REGISTRY.function` on it. Insert the seam BEFORE
the existing dispatch at **both** sites:

- `builder_values.rs:727` region (inline/raw path): before the
  `if native == Some("contains")…` chain, add
  `if let Some(r) = self.try_native_lower(target, args) { return r; }`.
- `builder_values.rs:1795` region (normal path): before the
  `match native_builtin_target(target)` arm, the same early-return.

With no function yet holding `Native`, this seam is inert → Phase-1 byte-identical
even before `get` is wired.

### 4.3 `src/codegen` module layout (end state of this plan)

```
src/codegen/
  mod.rs                         // pub mod registry; pub mod builtins;
  registry.rs                    // moved verbatim from src/builtins/descriptor.rs
                                 //   + NativeLower + Implementation::Native
  builtins/
    mod.rs                       // pub mod collections;
    collections/
      mod.rs                     // moved from src/builtins/collections.rs,
                                 //   minus get's entry (now in func_get)
      func_get.rs                // get's BuiltinFunction entry + doc consts (INTO_GET,
                                 //   DESC_GET) + Implementation::Native(lower_get)
                                 //   + fn lower_get(...)
```

- `src/main.rs`/`lib` root gains `pub mod codegen;`.
- `src/builtins/descriptor.rs` is deleted; its `pub(crate) use` re-exports (if any
  callers used `builtins::descriptor::X`) are replaced by `crate::codegen::registry::X`.
  31 files update imports (`super::descriptor` / `builtins::descriptor` /
  bare `descriptor::` → `crate::codegen::registry`).
- `src/builtins/collections.rs` is deleted; the 10 consumers of
  `builtins::collections::*` update to `crate::codegen::builtins::collections::*`.
  (Keep the public fn names identical — `is_collections_call`, `is_native_member_call`,
  `COLLECTIONS`, `resolve_call`, etc. — so only the path prefix changes.)

### 4.4 `get` migration (func_get.rs)

- Move the bodies of `lower_collection_get` into `func_get.rs` as a **free fn**
  `pub(crate) fn lower_get(b: &mut CodeBuilder, args: &[NirValue]) -> Result<ValueResult, String>`.
  It calls `b.lower_list_get(...)` / `b.lower_map_get(...)`.
- **Measured visibility surface:** `lower_get` needs, at minimum,
  `lower_list_get` and `lower_map_get` (and `is_provable_index_access`,
  `lower_value`, `allocate_stack_object` — whatever `lower_collection_get`'s body
  calls directly) to become `pub(crate)`. Before writing, run
  `grep -oE 'self\.[a-z_]+\(' src/target/shared/code/builder_collection_queries.rs`
  over the `lower_collection_get`/`lower_list_get`/`lower_map_get` span and promote
  exactly that set from `pub(super)` to `pub(crate)` — no wider. Record the promoted
  list in Corrections.
- `func_get.rs` owns `get`'s `BuiltinFunction` const (moved out of `mod.rs`'s
  `COLLECTIONS_FUNCTIONS` table), with `implementation: Implementation::Native(lower_get)`.
  `mod.rs`'s table references `func_get::GET`.
- Delete `lower_collection_get` and both ladder arms (`:727`, `:1795`). The seam
  from §4.2 now routes `get`.

## Compatibility / Format Impact

Nothing externally observable changes: identical CLI, identical man/spec output,
identical `.mfb` semantics, and **identical emitted `.ncode` on all targets**. The
only changes are internal module paths and the dispatch indirection. No golden is
re-baselined (a re-baseline would mean the refactor leaked — root-cause instead).

## Phases

Ordered uncertainty-first (prove the fn-pointer mechanism on `get` in place, moving
nothing) then blast-radius-last (relocations, old-symbol deletion). Every phase is
byte-neutral; the gate is the same each time.

> Tick `- [x]` in the same commit as the work; fill `Commit:` when it lands.

### Phase 1 — Prove the mechanism in place

Add `Implementation::Native` + the dual-path seam, and route `get` through it while
`lower_collection_get` still lives in `src/target` (the `Native` fn is a one-line
shim `|b, a| b.lower_collection_get(a)` — or a free fn that calls it). Nothing
moves. This falsifies the only real design uncertainty for a ~50-line diff.

- [x] `src/builtins/descriptor.rs`: added `NativeLower` type + `Implementation::Native`; hand-wrote `PartialEq`/`Eq` (`==` consumers exist — see Corrections) using `std::ptr::fn_addr_eq` for `Native`; added `BuiltinFunction::native_lower`; added `Native(_) => None` to `DefaultResolver::implementation_name`.
- [x] `builder_values.rs`: added `try_native_lower`; inserted the seam at both sites — early-return before the `native` ladder (`:722` region) and an `else if` inside the `raw_result_capture` wrapper (`lower_inline_builtin_raw`).
- [x] Pointed `collections.get`'s entry at `Implementation::Native(lower_get)` via a new `native_lowered` helper; `lower_get` is a free-fn shim delegating to the existing `lower_collection_get` (method item won't coerce — see Corrections). Ladder arms left in place (dead for `get`).
- [x] Promoted `CodeBuilder`, `ValueResult` (mod.rs) and `lower_collection_get` to `pub(crate)` so the registry can name/reference them.
- [x] Tests: `only_get_carries_native_lowering` in `descriptor.rs` — `get` has `native_lower()`, every other function does not.

Acceptance: MET — `bash scripts/artifact-gate.sh target/release/mfb collections` → `0 diff(s)` on all 5 targets; `cargo test --bin mfb` → 3836 passed, 0 failed.
Commit: ccf15b84b

### Phase 2 — Stand up `src/codegen`, move the registry

Create `src/codegen`; move `descriptor.rs` → `src/codegen/registry.rs` verbatim
(carrying the Phase-1 additions); update the 31 importers. Pure relocation.

- [x] Created `src/codegen/mod.rs` (`pub(crate) mod registry;`); added `mod codegen;` to `src/main.rs` (crate root — binary crate, no lib.rs).
- [x] `git mv src/builtins/descriptor.rs src/codegen/registry.rs`; removed `pub(crate) mod descriptor;` from `src/builtins/mod.rs` (same commit). `registry.rs` byte-identical to the old file → rename detected, history preserved.
- [x] Rewrote all importers (`super::descriptor::` + `crate::builtins::descriptor::` → `crate::codegen::registry::` tree-wide; bare `descriptor::` in `builtins/mod.rs` → same). No module-import (`use …descriptor;`) stragglers; 0 warnings.
- [x] The `NativeLower` type in `registry.rs` names `crate::target::shared::code::{CodeBuilder, ValueResult}` + `crate::target::shared::nir::NirValue` (accepted temporary `codegen→target` edge).
- [x] Fixed one dangling spec citation (see Corrections).

Acceptance: MET — `artifact-gate.sh collections` 0 diffs (all 5); `cargo test --bin mfb` 3836 passed, 0 failed.
Commit: 0bf877510

### Phase 3 — Move the `collections` package into `src/codegen`

Relocate the package module; update its consumers. Pure relocation.

- [x] Created `src/codegen/builtins/mod.rs` (`pub(crate) mod collections;`); `src/codegen/mod.rs` gained `pub(crate) mod builtins;`.
- [x] `git mv src/builtins/collections.rs → src/codegen/builtins/collections/mod.rs`; also `git mv collections_package.mfb` into the same dir (co-located so `include_str!("collections_package.mfb")` stays relative); removed `mod collections;` from `builtins/mod.rs`. `SOURCE_PATH` (logical sentinel) left unchanged.
- [x] Rewrote consumers → `crate::codegen::builtins::collections::*` (incl. the 4 bare `collections::` code sites in `builtins/mod.rs` and the `REGISTRY` static reference). Public fn names unchanged.
- [x] Rewrote the module's own `super::general::` (91) + `super::native_builtin_target` (1) → `crate::builtins::…` (collections left `builtins`). See Corrections.
- [x] Promoted 5 `general` resolver helpers `pub(super)` → `pub(crate)` (collections is no longer a `builtins` sibling). See Corrections.
- [x] Repointed 273 `collections.rs` + 141 `collections_package.mfb` doc citations to the new paths (see Corrections).

Acceptance: MET — `artifact-gate.sh collections` 0 diffs (all 5); `cargo test --bin mfb` 3836 passed, 0 failed; 0 warnings.
Commit: 3f5225ca8

### Phase 4 — Fully migrate `get`; delete the old symbol (largest blast radius last)

Move the real lowering into `func_get.rs`, promote the measured helper set to
`pub(crate)`, point `Native` at the free fn, delete `lower_collection_get` and both
ladder arms.

- [x] Measured the visibility surface (see Corrections) and promoted exactly it to `pub(crate)`: 8 `CodeBuilder` methods + `type_utils` module & its 2 type helpers + `ValueResult.{type_, location}` fields.
- [x] Created `src/codegen/builtins/collections/func_get.rs`: free `lower_get` (body moved verbatim, `self.`→`builder.`), `get`'s `BuiltinFunction` const `GET` (via `super::native_lowered`, with `INTO_GET`/`DESC_GET` moved here), `Implementation::Native(lower_get)`.
- [x] `collections/mod.rs`: `mod func_get;`; the table now references `func_get::GET`; dropped `get`'s inline entry, its `INTO_GET`/`DESC_GET`, the Phase-1 shim, and the now-unused `CodeBuilder`/`ValueResult`/`NirValue` imports.
- [x] Deleted `fn lower_collection_get` (builder_collection_queries.rs) and both `Some("get")` ladder arms in `builder_values.rs`; `get` now routes solely through the dual-path seam.
- [x] Repointed the `get.md` `lower_collection_get` citation → `func_get.rs:lower_get`.
- [x] `only_get_carries_native_lowering` (Phase 1) already asserts `get` resolves + is the sole `Native`; the type-resolution path (`native_builtin_target`) is untouched (verified green).

Acceptance: MET — `grep -rn 'fn lower_collection_get' src/target` → empty; the only `lower_collection_get` in `src` is a doc comment in `func_get.rs`. Byte-identical `artifact-gate.sh collections` 0 diffs (all 5). `cargo test --bin mfb` 3836 passed, 0 failed, 0 warnings. **Runtime proof:** a `collections::get` list+map program printed `10 / 30 / 36` as expected.
Commit: 1b7d2fa02

## Validation Plan

- Tests: existing `collections::`/`descriptor::` unit suites stay green; Phase 1
  adds the `native_lower` presence tests. No golden is re-baselined.
- Coverage check: the changed dispatch/lowering IS in the byte-identity fixture —
  `tests/byte-identity/collections` exercises `get` (list + map). Confirm the
  fixture's `src/main.mfb` calls `collections::get` (it does; grep it) so a green
  gate means "get's bytes are unchanged", not "get untested".
- Runtime proof: a `.mfb` program doing `io::print(toString(collections::get(...)))`
  on a list and a map, built and run, output identical to a `git stash`ed
  pre-plan build of the same program.
- Doc sync: none — man pages / spec unchanged (registry-driven `man2` output is
  identical since `get`'s descriptor fields are unchanged; verify
  `mfb man2 collections get` output is byte-identical before/after).
- Acceptance: full `cargo test --bin mfb` (0 failed) + one clean
  `bash scripts/artifact-gate.sh target/release/mfb collections` (0 diffs) after
  each phase; end-of-plan `rustup run 1.96.0 cargo fmt --all` per AGENTS.md.

## Open Decisions

- **When does `CodeBuilder` physically move to `src/codegen`?** — *Recommend: a
  dedicated later plan, after N functions have migrated and its surface is known*,
  vs. doing it in this plan. Measured basis for deferring: 126,330 lines / 40
  `impl` files / `ValueResult→Operand` + `NirValue=whole-NIR` cascade. This plan
  accepts the temporary `codegen↔target` cycle instead. (§3)
  Decision: Move later
- **Does `resolve_get` (type resolution) ride along into `func_get.rs`?** —
  *Recommend: not in this plan* (migrate lowering only; keep `get`'s type rule in
  the shared `CollectionsResolver`) vs. moving it now for true self-containment.
  "Owns the function" has two halves; this plan does the codegen half. (§3)
  Decision: if CollectionsResolver is truly shared it can stay out of `src/codegen/builtins` and be moved to `src/codegen/common` later
- **`Implementation` `PartialEq/Eq`** — RESOLVED (Phase 1): consumers exist, so
  kept `PartialEq/Eq`, hand-written with `fn_addr_eq` for `Native`. (Corrections)
- **Dispatch key string form** — RESOLVED (Phase 1): qualified (`"collections.get"`)
  at both sites. (Corrections)

## Corrections

- **Dispatch key form (§4.2 Open Decision): qualified.** `target` at both sites is
  the fully-qualified name (`"collections.get"`) — site 1 (`builder_values.rs:722`)
  feeds `native_builtin_target(target)`; site 2 (`lower_inline_builtin_raw`) uses
  `target.strip_prefix("bits.")` + `native_builtin_target(target)`. `REGISTRY.function(target)`
  keys on it directly. Verified by reading both sites.
- **`Implementation` `PartialEq/Eq` (§4.1 Open Decision): keep, hand-written.** The
  audit found real consumers — `assert_eq!(func.implementation, Implementation::Same)`
  in `os.rs:205`, `io.rs:202`, `money.rs:163`, `term.rs:628` — so they cannot be
  dropped. A *derived* `PartialEq` raises the fn-pointer-comparison lint; hand-wrote
  it, comparing `Native` via `std::ptr::fn_addr_eq` (all real consumers compare `Same`).
- **Method item won't coerce to `NativeLower`.** `CodeBuilder::lower_collection_get`
  (a method item) binds `CodeBuilder`'s lifetime from its impl and fails E0308
  against the higher-ranked `NativeLower`. A **free fn** with elided lifetimes
  coerces cleanly — Phase 1 uses a `lower_get` free-fn shim; Phase 4's real
  `lower_get` is a free fn for the same reason (already what §4.4 specifies).
- **Visibility promotions (Phase 1):** `CodeBuilder`, `ValueResult`
  (`src/target/shared/code/mod.rs`) and `lower_collection_get`
  (`builder_collection_queries.rs`) → `pub(crate)`. `NirValue` was already `pub(crate)`.
- **Doc-sync the move required (Phase 2, §Non-goals said "none"):** one dangling
  spec citation — `spec/architecture/09_modules.md` `[[src/builtins/descriptor.rs]]`
  → `[[src/codegen/registry.rs]]` (caught by `spec_citations_resolve`). The
  registry-move importer count was 31 files (matched §2); no other doc referenced
  the moved file.
- **Phase 3 was bigger than "10 consumers".** The move also required:
  (a) rewriting the module's own 91 `super::general::` + 1 `super::native_builtin_target`
  refs to `crate::builtins::…` (collections stays out of `builtins` but still calls
  its resolver helpers); (b) promoting 5 `general` helpers `pub(super)→pub(crate)`
  (`list_element`, `map_parts`, `element_accepts_item`, `set_element`,
  `function_parts` — `ResolvedCall` was already `pub(crate)`); (c) fixing 4 bare
  `collections::` code sites in `builtins/mod.rs`; (d) repointing **273**
  `src/builtins/collections.rs` and **141** `collections_package.mfb` doc citations
  (`man_citations_resolve` + `spec_citations_resolve`). Doc-sync is NOT "none" for
  a package move — the man pages cite the package source heavily.
- **Process note:** a two-pass `s/builtins::collections/codegen::builtins::collections/`
  perl collided with its own first pass (`crate::codegen::codegen::…`) and missed
  grouped/`bare` import forms. Fixed by collapsing `codegen::codegen::→codegen::`
  and a lookbehind prefix pass. Prefer a single lookbehind substitution for path moves.
- **Phase 4 measured visibility surface** (`pub(super)→pub(crate)`, no wider): the
  8 methods `lower_collection_get`'s body called — `is_provable_index_access`,
  `lower_value`, `allocate_stack_object`, `emit`, `store_value_at`, `lower_list_get`,
  `lower_map_get`, `materialize_owned_element`; the two free type helpers
  `list_element_type`/`map_type_parts` **and their `mod type_utils`** (was a private
  module); and **`ValueResult.{type_, location}` fields** (the struct was already
  `pub(crate)` from Phase 1, its fields were not). `abi` was already reachable
  (`crate::target::shared::abi`). `func_get` reaches `mod.rs`'s private
  `native_lowered`/`custom`/`req` via `super::` (child-of-module access, no promotion).
- **Doc-sync (Phase 4):** `get.md`'s `[[…builder_collection_queries.rs:lower_collection_get]]`
  citation repointed to `[[…func_get.rs:lower_get]]`. (The citation test checks the
  file, not the symbol, so the suite was green either way — fixed for accuracy.)

## Summary

The engineering risk is almost entirely front-loaded into Phase 1 — "can a `const`
`BuiltinFunction` carry an HRTB lowering fn pointer and can dual dispatch prefer it,
byte-identically?" — and it is answered for a ~50-line, nothing-moved diff against a
golden. Everything after is mechanical relocation (31-file + 10-file re-imports via
`git mv`) and a single measured visibility promotion, each gated byte-identical.
Left deliberately untouched: `CodeBuilder`'s 126k-line body (stays in `src/target`,
referenced), every non-`get` builtin (stays on the ladder), and `get`'s type
resolution (stays in the shared resolver). What ships is the *pattern* — registry-
owned lowering behind a permanent fallback seam — proven on one function, ready to
be applied to the next one as a small, byte-verified change.
