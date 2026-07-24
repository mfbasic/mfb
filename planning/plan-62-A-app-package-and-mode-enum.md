# plan-62-A: the `app::` package + the `Mode` enum + `--app` gating

Last updated: 2026-07-24
Overall Effort (AI): x-large (1d–3d) — the whole plan-62 feature (section A only)
Effort (Human): large (3h–1d)
Effort (AI): medium (1h–2h)
<!-- Diverges: a 17-file registration sweep mirroring 26 precedents + a source-companion enum +
     a CLI gate, and NO hardware proof (syntax accept/reject goldens only). Authoring dominates,
     so the AI is a band faster. -->

Depends on: nothing
Produces: the `app::` builtin package registered and typechecking; the `app::Mode`
enum with variants `Console` and `None`; the `getMode`/`setMode` call surface; and a
CLI compile error rejecting `IMPORT app` in any non-`--app` (console) build. Consumed by
plan-62-B (which adds the runtime state behind `setMode`) and by every later letter.

plan-62 makes **presentation mode** a first-class, extensible concept for `--app` builds:
a running program chooses what its window surface *is* through `app::setMode(...)`, instead
of the current tangle of a `uses_term` boolean plus `term::on`/`term::off`. This foundation
ships two modes — `Console` (the terminal-in-a-window that exists today) and `None`
(windowless) — and the machinery (enum, `setMode`, `--app` gating, static default, per-backend
teardown/rebuild) that a future `GUI` mode (plan-13) and `Canvas` mode plug into as new enum
variants with no change to the model.

**The single behavioral outcome of section A:** a program compiled `--app` that does
`IMPORT app` typechecks and can name `app::Mode::Console`, `app::Mode::None`,
`app::getMode()` and `app::setMode(m)`; the *same* program compiled without `--app` is
rejected at compile time with a clear "the `app` package requires app mode" diagnostic.
Nothing runtime-observable changes yet — `setMode` has no body until plan-62-B.

References (read first):

- `src/builtins/term.rs` — the fixed-arity, no-overload builtin-package shape `app::`
  mirrors. Find `is_term_call` / `resolve_call` / `param_types` by symbol.
- `src/builtins/datetime_package.mfb` (`EXPORT ENUM ZoneKind` at `:55`) and
  `src/builtins/money_package.mfb` (`EXPORT ENUM Rounding` at `:16`) — the **source-companion
  enum** precedent `app::Mode` follows. No reserved binary_repr ID is needed.
- `src/cli/build/mod.rs:166-193` — where `app_mode` / `build_mode` are computed, and
  `:177` — the `target_supports_app_mode` CLI reject that the `IMPORT app` gate mirrors.
- `src/builtins/mod.rs:29` — `is_builtin_import`, the name gate that makes `IMPORT app`
  legal; `src/resolver/packages.rs:10` — `IMPORT_PACKAGE_NOT_DECLARED`, the diagnostic
  today.

## Prerequisites  <!-- feature-wide: stated once here; B–E point back to this section -->

plan-62 has **no cross-plan prerequisite** — it is the foundation. The only conditions are
environmental, and all are already true.

| Must be true | Command | Status 2026-07-24 |
|---|---|---|
| App mode works on both target platforms | `ls src/target/macos_aarch64/app/ src/target/linux_gtk/` | **MET** |
| A GTK4 Linux box is reachable for backend proof (needed by D) | `grep -n 'GTK4' .ai/remote_systems.md` | **MET (box 2232)** |
| The `app` package name is free | `rg -n '"app"' src/builtins/mod.rs` → no `is_builtin_import` arm | **MET** |
| Source-companion enums are a working route | `rg -n 'EXPORT ENUM' src/builtins/*.mfb` | **MET (datetime, money)** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every
> command before continuing and again before deciding to stop. If you stop, report the status
> of every row. Locate every symbol below with `rg`, not by line number — this tree's
> citations have historically rotted by hundreds of lines in days (plan-13 master §2.3).

## Dependency graph  <!-- whole plan-62 feature -->

```
A ← nothing        app:: package + Mode enum + --app gating   (this doc)
B ← A              runtime mode state + static default + AppEntrySpec field
C ← B              macOS None-mode bootstrap + setMode transition
D ← B              GTK None-mode bootstrap + setMode transition   (fans out from B alongside C)
E ← C + D          term:: / io:: mode gating (wrong-mode errors)
```

Execution is topological order over this graph, re-checking each letter's stated
preconditions. C and D are a genuine fan-out — both consume B's shared seam and neither
depends on the other; **the shared surface (B) is deliberately not bundled with its first
backend** (write-plan skill: never bundle a shared surface with its first backend).

## 1. Goal

- `app` is registered as the 27th builtin package (`ls src/builtins/*.rs | wc -l` → 26 today,
  minus `mod.rs`), importable **only** in `--app` builds.
- `app::Mode` is an enum with exactly two variants for now: `Console`, `None`. It is declared
  in a new `src/builtins/app_package.mfb` source companion (the datetime/money route), so it
  needs **no** reserved `binary_repr` type ID.
- `app::getMode()` returns `app::Mode` (no arguments). `app::setMode(m As app::Mode)` returns
  `Nothing`. Both are fixed-arity, no overloads — the `term::` shape.
- `IMPORT app` in a non-`--app` build is a **compile-time error** with a diagnostic naming
  app mode, raised at the CLI before lowering (mirroring `target_supports_app_mode`).

### Non-goals (explicit constraints)

- **No runtime behavior.** `setMode` gets no helper body, no state slot, no window teardown
  here — that is plan-62-B/C/D. Section A is accept/reject only.
- **No `GUI`/`Canvas` variants.** Those are plan-13 and a future plan. `app::Mode` ships with
  `Console` and `None` and nothing else.
- **Do not thread `NativeBuildMode` into syntaxcheck.** Research showed `check_project_collect`
  has no build mode and giving it one touches every caller (§3.3, Open Decision 1). The gate
  lives at the CLI.
- **Do not change any existing package**, `term::`, or the `LINK`/FFI surface.

## 2. Current State

`app::` does not exist: `ls src/builtins/app.rs` → no such file; `rg -o '"app\.[a-z]' src/`
→ 0. `IMPORT app` today produces `IMPORT_PACKAGE_NOT_DECLARED` (`src/resolver/packages.rs:10`)
because `"app"` is absent from `is_builtin_import` (`src/builtins/mod.rs:29`).

The build mode is a compile-time three-value `NativeBuildMode { Console, MacApp, LinuxApp }`
(`src/target.rs:37`), with `is_app()` (`:57`) true for the two app toolkits. **This is a
different axis from the runtime `app::Mode` this plan introduces** — see §3.1. `app_mode` is
computed at `src/cli/build/mod.rs:166` as `options.app_mode || build_mode_is_app(&manifest)`
(the `-app` CLI flag is additive over the manifest `"mode":"app"`), and `build_mode` at `:187`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Files to touch to register one builtin package | **17** | `rg -l 'builtins::term\b\|builtins::net\b' -g '*.rs' src/ \| wc -l` |
| Builtin packages today | **26** | `ls src/builtins/*.rs \| wc -l` (minus `mod.rs`) |
| Source-companion `EXPORT ENUM` precedents | **≥3** | `rg -n 'EXPORT ENUM' src/builtins/datetime_package.mfb src/builtins/money_package.mfb` |
| Existing builtin *enum* with a reserved high-range type ID | **0** | `rg -n '0xffff_fe' src/binary_repr/mod.rs` — all reserved builtin types are records |
| Package sizes, for scale | `term.rs` 331, `net.rs` 753 | `wc -l src/builtins/{term,net}.rs` |

The 17-file list, grouped (all confirmed by `rg -l` above):

- **Name gate:** `src/builtins/mod.rs` — `is_builtin_import` (`:29`), `is_builtin_type` (`:57`),
  and `pub(crate) mod app;`.
- **Resolver:** `src/resolver/mod.rs` — the `_TYPES` array (`:28`) and the source-companion
  augment chain (`resolve_project_with`, `:72`).
- **Syntaxcheck:** `src/syntaxcheck/{builtins.rs (BUILTIN_PACKAGES :53), mod.rs (:163),
  inference.rs (:821), helpers.rs (:296)}`.
- **IR:** `src/ir/lower.rs` (`:66`, `:1900`, `:3557`), `src/ir/verify/mod.rs` (`:1081`,
  `:1184`), `src/ir/verify/compat.rs` (`:188`, `:311`).
- **binary_repr:** `src/binary_repr/{sections.rs, builder.rs, tests.rs}` — **only if** any
  `app::` type needs a reserved ID. With the source-companion enum route, `Mode` uses the
  normal user-enum path and these need **no** edit (see §3.2). Kept in the sweep list only so
  the count matches the census; expected to be no-ops in A.
- **Targets:** `src/target/{macos_aarch64,linux_common}/plan.rs`, `src/target/shared/code/mod.rs`,
  `src/target/shared/code/{module_analysis.rs, validation.rs}`, `src/target/shared/runtime/mod.rs`.
  Most of these carry *runtime-helper* wiring that is plan-62-B's concern; in A they get the
  name/dispatch arm only where a call must resolve.

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| A `.mfb` source-companion enum needs no reserved binary_repr ID | **CONFIRMED** | `datetime_package.mfb:55` enums flow through `BinaryReprExportKind::Enum` (`sections.rs:44`), getting `FIRST_TABLE_TYPE_ID`-range IDs like any user enum |
| Syntaxcheck cannot see the build mode | **CONFIRMED** | `check_project_collect(project_dir, ast)` (`syntaxcheck/mod.rs:150`) and `check_project` (`:180`) take no `NativeBuildMode`/target/flag |
| The CLI knows the full `app_mode` before syntaxcheck runs | **CONFIRMED** | `app_mode`/`build_mode` computed at `cli/build/mod.rs:166-193`; `check_project_collect` called at `:328` with `build_mode` in scope but unused |
| There is CLI precedent for a build-mode reject with a clear diagnostic | **CONFIRMED** | `cli/build/mod.rs:177` rejects `-app` on unsupported targets via `target_supports_app_mode` |
| `app::` package exists in any form | **FALSE** | no `builtins/app.rs`, 0 `app.*` calls |

## 3. Design Overview

Three independent pieces: the source-companion enum, the package registration sweep, and the
CLI gate. Design uncertainty was concentrated in the last two and has been **retired by
research** (the enum route and the CLI-gate feasibility are both confirmed above), so A carries
little risk; its bulk is a mechanical 17-file sweep with 26 precedents.

### 3.1 The two `Console`s — keep them separate

`NativeBuildMode::Console` (`src/target.rs:39`) is **compile-time**: "this is a plain terminal
binary, not an `--app` build." `app::Mode::Console` (this plan) is **runtime**: "this `--app`
window is presenting the terminal surface." They only coexist inside an `--app` build; a
`NativeBuildMode::Console` build has no `app::` package at all (it is gated out here) and thus
no `app::Mode`. To stop compiler code confusing the two, the **Rust-internal** enum introduced
in plan-62-B should be named distinctly (e.g. `PresentationMode`), while the **user-facing**
name stays `app::Mode` (Open Decision 2).

### 3.2 `app::Mode` as a source-companion enum

Create `src/builtins/app_package.mfb` with `EXPORT ENUM Mode` (variants `Console`, `None`),
mirroring `datetime_package.mfb`. Wire it exactly as datetime wires its companion:
`source_file()` via `include_str!`, `uses_package`, and `augmented_project` pushing the file
into the augment chain (`resolver/mod.rs`, `syntaxcheck/mod.rs:163`, `ir/lower.rs:66`). The
enum then flows through the ordinary user-enum path and receives a `FIRST_TABLE_TYPE_ID`-range
wire ID — **no `binary_repr/mod.rs` reserved ID, no `sections.rs`/`reader.rs` arm.**

**Rejected alternative — a Rust-side reserved-ID enum** (like `term::TermColor`'s record):
rejected because there is no builtin-enum precedent for that route (§2 population = 0), it
forces `binary_repr` edits, and `Mode` never crosses the native ABI as a record — it is a
plain discriminant.

### 3.3 The `--app` gate at the CLI

After `app_mode`/`build_mode` are known (`cli/build/mod.rs`, post-`:193`), scan the resolved
AST's imports for `"app"` and, when `!app_mode`, emit a compile error naming app mode — the
same shape as the `:177` target reject. This is a genuine compile-time diagnostic that fires
before lowering.

**Rejected alternative — thread `NativeBuildMode` into `check_project_collect`:** rejected as
Open Decision 1. It touches every syntaxcheck caller for one package's benefit, and the
resolver's manifest (`resolve_project(project_dir, manifest, ast)`) sees only manifest-declared
app mode, not the additive `-app` flag — so only the CLI has the full picture anyway.

## Compatibility / Format Impact

- **New:** the `app::` package (27th builtin); `app::Mode` enum {`Console`, `None`} via a
  source companion; a CLI compile error for `IMPORT app` without `--app`.
- **Unchanged:** every existing package; `term::`; `is_c_abi_type`/FFI; the `LINK` surface;
  all runtime behavior (A adds none).

## Phases

> **NOTE — keep the checkboxes current as you go; tick `- [x]` in the same commit as the work.
> An unticked box means NOT DONE.**

### Phase 1 — the `Mode` enum companion (lowest uncertainty first, but it is the type everything names)

- [ ] Create `src/builtins/app_package.mfb` with `EXPORT ENUM Mode` (`Console`, `None`),
      following `datetime_package.mfb`.
- [ ] Add `src/builtins/app.rs` with `source_file()`/`uses_package()`/`augmented_project()`
      mirroring `datetime.rs:359-375`; declare `pub(crate) mod app;` in `builtins/mod.rs`.
- [ ] Wire the companion into the augment chains: `resolver/mod.rs`, `syntaxcheck/mod.rs:163`,
      `ir/lower.rs:66`.

Acceptance: a `--app` program that does `IMPORT app` and binds `DIM m As app::Mode = app::Mode::Console`
typechecks; `app::Mode::None` resolves; a golden in `tests/syntax/app/` captures it.
Commit: —

### Phase 2 — the `getMode`/`setMode` surface (the 17-file registration sweep)

- [ ] Register `app` in `is_builtin_import` (`builtins/mod.rs:29`) and `is_builtin_type`
      (`:57`).
- [ ] In `app.rs`, declare `GET_MODE = "app.getMode"` / `SET_MODE = "app.setMode"`, plus
      `is_app_call`, `param_types` (`getMode → []`, `setMode → ["Mode"]`), `call_return_type_name`
      (`getMode → Mode`, `setMode → Nothing`), `arity`, `expected_arguments`, `resolve_call` —
      the `term.rs` shape.
- [ ] Add the `app` row to `syntaxcheck::BUILTIN_PACKAGES` (`builtins.rs:53`) mirroring the
      `net` row; add `app::` arms in `inference.rs:821`, `helpers.rs:296`, `ir/lower.rs:1900`/`:3557`,
      `ir/verify/mod.rs:1081`/`:1184`, `ir/verify/compat.rs:311`.
- [ ] Confirm the `binary_repr` files (`sections.rs`, `builder.rs`, `tests.rs`) need **no**
      edit under the source-companion route; if a golden round-trip references the new enum,
      seed it. Note the result in the plan (moot vs. touched).

Acceptance: a program naming `app::getMode()` and `app::setMode(app::Mode::None)` typechecks;
`cargo check --all-targets` is clean; a `tests/syntax/app/` golden covers arity and a
wrong-arity rejection (`app::setMode()` with no arg → `TYPE_CALL_ARITY_MISMATCH`).
Commit: —

### Phase 3 — the `--app` gate (blast radius: rejects a whole build — but a pure diagnostic)

- [ ] In `src/cli/build/mod.rs` after `:193`, scan `concrete_ast` imports for `"app"`; when
      `!app_mode`, emit a compile error naming app mode (mirror the `:177` reject). Add the
      diagnostic to the error catalog if one is required.
- [ ] Tests: `tests/syntax/app/` — `IMPORT app` under `--app` accepted; the CLI-level reject
      for a console build (whichever harness exercises CLI build-mode rejects; follow the
      `target_supports_app_mode` test's location).

Acceptance: `IMPORT app` compiled `--app` builds; compiled without `--app` it fails with a
diagnostic that names app mode; both are captured by tests. The reject is a compile error, not
a lowering/link failure.
Commit: —

## Validation Plan

- Tests: `tests/syntax/app/` (golden-backed, in the gate denominator per plan-13 §Validation).
  Negative cases — wrong arity, `IMPORT app` in console mode — are the substance; A creates
  nothing that runs.
- Coverage check: confirm `tests/syntax/app/` lands in the gate's denominator; `tests/acceptance/`
  has no `golden/` dir by design — do not put proofs there.
- Runtime proof: none — nothing runs until plan-62-B/C/D.
- Doc sync: a new `src/docs/spec/stdlib/` topic for `app::` and `src/docs/man/builtins/app/`
  pages per `.ai/man_package_template.md` — **not** `src/docs/spec/package/` (that is the binary
  container format; plan-13 master §2.5).
- Acceptance: `scripts/test-accept.sh` green.

## Open Decisions

1. **Where the `--app` gate lives** — recommended the **CLI check** (§3.3), matching
   `target_supports_app_mode`, vs. threading `NativeBuildMode` into syntaxcheck (touches every
   caller; can't see the additive `-app` flag from the manifest alone).
2. **Rust-internal name for the presentation-mode enum** — recommended `PresentationMode` in
   compiler source to avoid confusion with `NativeBuildMode::Console`, while the user-facing
   name stays `app::Mode::Console` (§3.1). Decided in plan-62-B where the Rust enum is born.
3. **Is `app` helper-backed or native-direct?** `setMode` must do real runtime work
   (state write now; window teardown in C/D), so **helper-backed** (like `term::`) — decided
   here, implemented in B. This determines that a `_specs.rs`/catalog entry is needed (B §5).

## Corrections

<!-- Filled in during execution. -->

- 2026-07-24 — plan-13 master §2.4 cites `AppEntrySpec` at `types.rs:636-641`; it is now at
  `types.rs:840-845` (rotted). Recorded here because plan-62-B extends that struct.

## Summary

Section A is a low-risk foundation: a source-companion enum (a route with three precedents and
zero binary_repr cost), a 17-file registration sweep with 26 precedents, and a CLI gate that
copies an existing reject. All the design uncertainty — the enum route, and whether `--app`
gating can be a clean compile error — was retired by research before this was written. What is
left untouched: every other package, `term::`, the FFI/`LINK` surface, and anything that runs.
