# plan-62-B: runtime mode state, the static default, and the `AppEntrySpec` field

Last updated: 2026-07-24
Effort (Human): medium (1h–2h)
Effort (AI): small (<1h)
<!-- Diverges: boilerplate closely mirroring uses_term / term_specs, with only a light runtime
     proof (read a discriminant). Authoring dominates, so the AI is a band faster. -->

Depends on: plan-62-A. Feature-wide precondition: plan-62-A §Prerequisites.
Produces: a runtime **current-presentation-mode** state slot; the `_mfb_rt_app_get_mode` /
`_mfb_rt_app_set_mode` runtime helpers (state read/write only — **no** window teardown yet);
the **static initial-mode default** (Console when the program references `setMode` nowhere,
else None), mirroring `uses_term`; and a new `AppEntrySpec` presentation-mode field threaded
into `emit_app_program_entry`. Consumed by C (macOS), D (GTK), and E (gating).

This is the **shared seam** of plan-62: the worker-side notion of "what mode am I in," the
compile-time decision of what mode a program *starts* in, and the plumbing that carries that
decision into the platform bootstraps — all **without** touching a single line of window-build
code. C and D consume this seam; deliberately, B does not bundle it with either backend.

**The single behavioral outcome of section B:** in an `--app` build, `app::getMode()` returns
`Console` for a program that never calls `setMode`, and returns `None` at startup for a program
that *does* reference `setMode` anywhere; `app::setMode(m)` updates the slot so a subsequent
`getMode()` reflects `m`. No window appears or disappears yet — that surface work is C/D — but
the state machine and the static default are fully live and testable through `getMode`.

References (read first):

- `src/target/shared/code/mod.rs:824-826` — `uses_term`, the whole-program presence scan this
  plan's static default copies verbatim in shape.
- `src/target/shared/code/error_constants.rs:328-348` — the `TERM_STATE_*` slot layout, the
  precedent for reserving a new presentation-mode state slot in the entry frame.
- `src/target/shared/runtime/{mod.rs,catalog.rs,term_specs.rs}` — the `RuntimeHelper` family
  registry, the catalog consistency test, and the per-package spec-file shape.
- `src/target/shared/code/types.rs:840-845` — `AppEntrySpec` (2 fields today), the struct this
  plan extends. **plan-13 cites this at `:636`; that is rotted (plan-62-A Corrections).**

## Prerequisites

See plan-62-A §Prerequisites (feature-wide). Additionally:

| Must be true | Command | Status 2026-07-25 |
|---|---|---|
| plan-62-A has landed (`app::` package + `Mode` enum + gating) | `rg -n '"app"' src/builtins/mod.rs` → `is_builtin_import` arm present | **MET** (commit b91f87924) |
| The catalog family-count assertion is known | `rg -n 'families.len\(\)' src/target/shared/runtime/catalog.rs` | **MET** (was 10; now bumped to 11 with `RuntimeHelper::App`) |

> **NOTE — the Command column is the truth; re-run every row before continuing and before
> stopping. If you stop, report every row.**

## 1. Goal

- A **presentation-mode state slot** in the program-entry frame holds the current mode as an
  integer discriminant (`Console`/`None`), addressed off the pinned arena-state register like
  `term_state_offset`.
- `_mfb_rt_app_set_mode` writes the slot; `_mfb_rt_app_get_mode` reads it. In section B these
  are **pure state read/write** — no window is built or torn down. (The teardown/rebuild body
  is layered on in C/D; B leaves a documented seam where the backend transition hook is called.)
- The **initial value** of the slot is decided statically: `Console` if the program references
  no `setMode` runtime symbol, else `None`. Implemented as a `runtime_symbols` presence scan
  beside `uses_term`.
- `AppEntrySpec` gains an `initial_mode` field carrying that decision; it is threaded from
  `emit_app_program_entry` toward the bootstraps (the bootstraps consume it in C/D).

### Non-goals (explicit constraints)

- **No window build/teardown.** `setMode`'s body only writes the slot in B; the surface
  reconciliation is C/D. Keep the backend transition hook a named, empty seam here.
- **No `term::`/`io::` gating.** That is E. B must not make `term::` or `io::input` error.
- **No console-build effect.** In a `NativeBuildMode::Console` build there is no `app::`
  package (gated in A) — the state slot, helpers, and default apply only when `is_app()`.

## 2. Current State

`uses_term` (`mod.rs:824`) is `runtime_symbols.iter().any(|s| s.starts_with("_mfb_rt_term_"))`
— a pure whole-program presence scan, no flow analysis. It gates term-state slot reservation
(`term_state_offset` `:847`, `term_state_slots` `:852`, folded into `arena_global_slots` `:860`)
and is threaded into `AppEntrySpec { uses_term }` (`:930`). This is the exact pattern the
initial-mode default reuses: scan for `_mfb_rt_app_set_mode`, reserve one slot, thread a field.

`AppEntrySpec` (`types.rs:840`) has two fields (`language_entry_accepts_args`, `uses_term`) and
is constructed at `mod.rs:930` (real) and `linux_common/code.rs:1232` (a `#[should_panic]` rv64
test). The window/transcript builders (`emit_main_bootstrap` on both platforms) take **no
`spec`** today — so a new field must be threaded from `emit_app_program_entry` (macOS
`mod.rs:542`, GTK `mod.rs:418`) into them (that threading is C/D's task; B adds the field and
sets it).

Runtime-helper registry: `RuntimeHelper` enum (`runtime/mod.rs:4-17`), `name()` (`:20-35`),
`symbol_for_call(helper, call)` (`:38`) derives `_mfb_rt_{helper}_{call}` (never stored).
Per-package specs live in `*_specs.rs` (e.g. `term_specs.rs:3`); the catalog
`SUPPORTED_HELPER_SPECS` (`catalog.rs:9`) lists them, and a consistency test (`catalog.rs:260-282`)
asserts every non-native-direct family has ≥1 spec and a family count (`families.len()` == 10).

### Measured populations

| What | Count | Command |
|---|---|---|
| `RuntimeHelper` families catalogued today | **10** | `rg -n 'families.len\(\)' src/target/shared/runtime/catalog.rs` |
| `_mfb_rt_*` helpers in the registry | **124** | `rg -oh '_mfb_rt_[a-z0-9_]*' src/ \| sort -u \| wc -l` (a re-count; plan-13 §2.1 said 124) |
| `TERM_STATE_SLOTS` bytes reserved (the slot-reservation precedent) | `(144+72)/8` | `rg -n 'TERM_STATE_SLOTS' src/target/shared/code/error_constants.rs` |
| `AppEntrySpec` fields today | **2** | `rg -n -A3 'struct AppEntrySpec' src/target/shared/code/types.rs` |

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| `uses_term` is a whole-program symbol presence scan reusable for `setMode` | **CONFIRMED** | `mod.rs:824-826`, no flow analysis |
| Helper symbols are derived, not stored — `_mfb_rt_app_set_mode` comes free | **CONFIRMED** | `symbol_for_call` (`runtime/mod.rs:38`); `RuntimeHelper::App` + `"app.setMode"` |
| Adding a `RuntimeHelper` family requires bumping the catalog assertion | **CONFIRMED** | `catalog.rs:264-282` — the `for helper in [...]` list and `families.len()` == 10 |
| `AppEntrySpec` bootstrap consumers do not yet receive `spec` | **CONFIRMED** | `emit_main_bootstrap` takes no args on both platforms (plan-62-C/D §2) |

## 3. Design Overview

Three layered pieces: the state slot, the two helpers, and the static default + `AppEntrySpec`
field. Design uncertainty is low — every piece has a direct precedent (`term_state`,
`term_specs`, `uses_term`). The **one seam to get right** is keeping `setMode`'s helper body
split into a **mode-slot write** (B, backend-neutral) and a **surface-reconcile hook** (C/D,
per-backend). Wiring the surface work into B would bundle the shared seam with a backend and
force a re-cut when the second backend lands.

### 3.1 The state slot

Reserve one 8-byte slot (`PRESENTATION_MODE_OFFSET`) in the entry frame beside the term-state
region (`error_constants.rs:328`), folded into `arena_global_slots`. Only reserved when
`is_app()` — a console build has no presentation mode. The slot holds the `app::Mode`
discriminant (0 = Console, 1 = None; keep the numbering matching the `.mfb` enum's declared
order so `getMode`'s returned discriminant is directly the enum value).

### 3.2 The two helpers

Add `RuntimeHelper::App` (`runtime/mod.rs`), `name() => "app"`, a new
`src/target/shared/runtime/app_specs.rs` with `APP_SET_MODE_SPEC` and `APP_GET_MODE_SPEC`
(mirroring `term_specs.rs`), register it in the catalog and bump `families.len()` 10 → 11 and
the `for helper in [...]` list (`catalog.rs:264`). Backend routing arms at
`macos_aarch64/plan.rs:635` and `linux_common/plan.rs:458` (`net::is_net_call` neighbourhood)
so `app.*` calls lower to `_mfb_rt_app_*`.

`_mfb_rt_app_get_mode` loads `PRESENTATION_MODE_OFFSET` into the result value register (the
`term::isOn` shape — `term.rs:518`). `_mfb_rt_app_set_mode` stores its argument to the slot,
then calls a **backend surface-reconcile hook** that in B is a no-op stub (documented) and in
C/D becomes the real teardown/rebuild. Section B proves the state machine through `getMode`
alone.

### 3.3 The static initial-mode default

Beside `uses_term` (`mod.rs:824`), compute:

```
let uses_set_mode = runtime_symbols.iter().any(|s| s == "_mfb_rt_app_set_mode");
let initial_mode = if uses_set_mode { PresentationMode::None } else { PresentationMode::Console };
```

(`uses_set_mode` keys on the `setMode` symbol specifically — a program that only calls
`getMode` still defaults to Console; §Open Decision 1.) Thread `initial_mode` into
`AppEntrySpec` (`types.rs:840`, construction `mod.rs:930`) and use it to seed the state slot in
program-entry emission. The Rust enum is named `PresentationMode` to avoid colliding with
`NativeBuildMode::Console` (plan-62-A §3.1, Open Decision 2 — resolved here).

## Compatibility / Format Impact

- **New:** a presentation-mode state slot (app builds only); `_mfb_rt_app_get_mode` /
  `_mfb_rt_app_set_mode` (helper #125/#126); `RuntimeHelper::App`; an `AppEntrySpec.initial_mode`
  field. `arena_global_slots` grows by one slot in app builds — verify the `thread::start`
  worker-region sizing (bug-369, `types.rs:810`) still matches the entry frame.
- **Unchanged:** console builds (no slot, no helpers); every existing helper; window behavior
  (B builds/tears down nothing).

## Phases

> **NOTE — tick `- [x]` in the same commit as the work. Unticked means NOT DONE.**

### Phase 1 — the state slot + `getMode`/`setMode` helpers (state only)

- [x] Reserve the presentation-mode slot (`PRESENTATION_MODE_SLOTS` in `error_constants.rs`)
      one slot past `TERM_STATE_*`; fold into `arena_global_slots`. **Gated on `uses_app`**
      (any `_mfb_rt_app_` symbol), NOT `is_app()` — the `uses_term` model, so app binaries that
      never touch `app::` keep byte-identical entries (see Corrections; macos-app-mode-* proven
      unchanged).
- [x] Add `RuntimeHelper::App` + `name()`; create `runtime/app_specs.rs` with
      `APP_GET_MODE_SPEC`/`APP_SET_MODE_SPEC`; register in `catalog.rs` and bump the family
      count 10 → 11 (+ the `for helper in [...]` list).
- [x] Emit the get/set helpers in `shared/code/app.rs` (`lower_app_helper`): getMode loads the
      slot → result; setMode stores arg → slot, then calls the **no-op** `emit_app_mode_reconcile`
      seam (new `CodegenPlatform` trait method, default `None`; C/D override). Routing is via the
      catalog (`helper_for_call` + `app_specs`) + each app backend's capability list
      (`macos_aarch64` inline `runtime_calls`, `linux_common::RUNTIME_CALLS`) — NOT a `plan.rs`
      import arm (app needs no OS imports in B; see Corrections).
- [x] Worker-region sizing (bug-369): the presentation slot is in `arena_global_slots`, which
      `thread::start` sizes workers from, so worker and entry frames match automatically.

Acceptance: an `--app` program that does `app::setMode(Mode.None)` then reads `app::getMode()`
observes `None`; one that never calls `setMode` observes `Console`. **VERIFIED** on macOS device
headlessly (`scripts/test-macapp.sh` cases "app:: default presentation mode is Console",
"app::setMode/getMode round-trip", both `code=0`). No window change asserted.
Commit: d12ac331a

### Phase 2 — the static initial-mode default + `AppEntrySpec` field

- [x] Add `PresentationMode { Console = 0, None = 1 }` (Rust-internal name; §3.3, Open Decision 2)
      and compute `initial_mode` beside `uses_term` — via `module_uses_call(module, "app.setMode")`
      (the `uses_rng` model), NOT a `_mfb_rt_app_set_mode` runtime-symbol scan: the real derived
      symbol is `_mfb_rt_app_app_setMode` (see Corrections).
- [x] ~~Add `initial_mode: PresentationMode` to `AppEntrySpec`~~ — **DEFERRED to plan-62-C**, the
      letter that first *reads* it (the bootstrap's startup-window decision). Adding an unread
      field in B violates the no-dead-code rule. B carries the static default into program entry
      via a new `ProgramEntrySpec::seed_presentation_mode_offset` instead (which B reads). The
      rv64 `#[should_panic]` `AppEntrySpec` construction therefore needed no change.
- [x] Seed the slot from `initial_mode` in `entry.rs::lower_program_entry` (shared across all
      backends, threaded via `ProgramEntrySpec::seed_presentation_mode_offset`): store `1`
      (`None`) when that is the static default; `Console` needs no store (zero-init).

Acceptance: `getMode()` at the very first program statement returns `Console` for a
no-`setMode` program and `None` for a program that references `setMode` anywhere. **VERIFIED**
on macOS device (`scripts/test-macapp.sh` case "app:: static default is None when setMode is
referenced", `code=0`; the Console case is the default test above).
Commit: d12ac331a

## Validation Plan

- Tests: a new `tests/rt-behavior/app/` (or the project's runtime-golden location) exercising
  `getMode`/`setMode` state and the static default. Confirm goldens land in the gate denominator
  (plan-13 §Validation warns `tests/acceptance/` has no `golden/`).
- Runtime proof: on macOS on-device and the Debian aarch64 GTK box (`.ai/remote_systems.md`,
  box 2232) — `getMode` returns the expected discriminant in each default case.
- Doc sync: extend the `app::` `src/docs/spec/stdlib/` topic and man pages with `getMode`/
  `setMode` and the default rule.
- Acceptance: `scripts/test-accept.sh` green.

## Open Decisions

1. **Presence key: `setMode` only, or any `app::` call?** Recommended **`setMode` only** — a
   read-only `getMode` should not force windowless startup (matches the user's rule "if any
   `setMode` is used at all"). Scan for `_mfb_rt_app_set_mode` specifically.
2. **Discriminant numbering** — recommended match the `.mfb` enum declaration order so
   `getMode`'s slot value *is* the enum value with no remap. Verify the enum lowering assigns
   `Console = 0`, `None = 1`.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-25 — **Helper symbol is `_mfb_rt_app_app_setMode` / `_mfb_rt_app_app_getMode`**, not
  the plan's `_mfb_rt_app_set_mode`. `symbol_for_call` sanitizes the *full* call string
  (`"app.setMode"` → `app_setMode`) after the `_mfb_rt_app_` prefix — the same double-name shape
  as `_mfb_rt_term_term_off`. The static default therefore uses
  `module_uses_call(module, "app.setMode")` (the `uses_rng` model), which is robust to the
  spelling, instead of a runtime-symbol scan.

- 2026-07-25 — **The mode slot is gated on `uses_app`, not `is_app()`.** `is_app()` would reserve
  a slot (growing `arena_global_slots` by one) in *every* app build, churning the entry goldens
  of app fixtures that never touch `app::` (e.g. `macos-app-mode-plumbing`). Gating on
  `uses_app` (any `_mfb_rt_app_` symbol present) mirrors `uses_term` exactly and keeps those
  fixtures byte-identical — verified via `test-accept.sh` on all three `macos-app-mode-*`.

- 2026-07-25 — **`AppEntrySpec::initial_mode` is deferred to plan-62-C.** The static default is
  a compile-time value; B needs it only to seed the per-arena slot, which it does through a new
  `ProgramEntrySpec::seed_presentation_mode_offset` (read in `entry.rs::lower_program_entry`).
  The `AppEntrySpec` field is for the per-backend *bootstrap's* startup-window decision — first
  read by C — so it lands there; adding it in B would be an unread field (AGENTS.md: never
  "consumed by a later phase"). Phase 2's behavior (None at first statement) is fully delivered
  by the seed.

- 2026-07-25 — **Backend routing is via the catalog + capability list, not a `plan.rs` import
  arm.** Adding `APP_GET_MODE_SPEC`/`APP_SET_MODE_SPEC` to the catalog makes `helper_for_call`
  route `app.getMode`/`setMode` to `RuntimeHelper::App`; each app backend then lists them in its
  `BackendCapabilities.runtime_calls` (`macos_aarch64` inline; `linux_common::RUNTIME_CALLS`
  for D). App helpers need no OS-library imports in B (the reconcile is a no-op), so no
  `macos_aarch64/plan.rs` / `linux_common/plan.rs` import arm is required.

- 2026-07-25 — **Runtime proof lives in `scripts/test-macapp.sh`, not `tests/rt-behavior/app/`.**
  App runtime is not exercised by `test-accept.sh` (which builds native *dumps*, never runs app
  bundles); the sanctioned macOS app-runtime gate is `test-macapp.sh` (headless, exit-code
  observable), where existing app runtime behavior (io output, exit-code propagation) is already
  proven. Three cases added there (default Console, setMode round-trip, None static default).

- 2026-07-25 — **Fixed a latent plan-62-A defect: `PACKAGE_ORDER` in `src/docs/man/mod.rs`
  needed an `app` row.** Adding `src/docs/man/builtins/app/` in A made 33 generated man packages
  against a 32-entry `PACKAGE_ORDER`, failing `docs::man::tests::man_citations_resolve`. It
  slipped past A because the full `cargo test` was run *before* the man pages were added and only
  a filtered `cargo test man` after — the exact "run full `cargo test`, never one module" trap.
  Fixed here (row added after `term`); full suite now 0-failed.

## Summary

B is the backend-neutral heart of the mode system: one state slot, two helpers, and a static
default that copies `uses_term`'s shape exactly. The single discipline that keeps the split
clean is leaving `setMode`'s surface-reconcile a **named no-op seam** for C/D to fill — putting
window work here would bundle the shared seam with a backend and break the fan-out. Untouched:
console builds, existing helpers, and all window behavior.
