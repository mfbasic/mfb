# plan-13-F: the GTK4 backend

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: plan-13-D (the solver) **and** plan-13-E (the seam contract). **Not on the
macOS backend's widgets** — it implements the same contract independently.
Feature-wide precondition: plan-13 master §Prerequisites.
Produces: the GTK4 implementation of the 26-op seam, flavor-correct GTK imports.
Consumed by 13-J.

Implements the same seam on GTK4, reusing the emitted solver unchanged.

The single behavioral outcome: the canonical program runs on the Debian aarch64 box with
frames **identical to macOS and to the headless host**.

**This unit is a fan-out sibling of 13-E, not its successor.** The 2026-07-09 draft
numbered it Phase 5 after macOS's Phase 3, while its own text says it "reuses the emitted
`_mfb_rt_app_layout` unchanged". Both consume 13-D; neither consumes the other.

References (read first):

- `src/target/linux_gtk/` — 3417 LOC of existing GTK4 app-mode. Find the signal wiring
  with `rg -n 'g_signal_connect_data' src/target/linux_gtk/bootstrap.rs` and the idle-post
  with `rg -n 'g_idle_add' src/target/linux_gtk/app_io.rs`.
- `src/target/linux_gtk/mod.rs` — the GTK4 soname binding. Find with `rg -n 'libgtk-4'`.
- `planning/old-plans/plan-51-*.md` and `plan-56-*.md` — **AppImage per libc flavor and
  flavor-aware GTK imports.** §3.1; the 2026-07-09 docs predate both.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-13-D has landed | `rg -n '_mfb_rt_app_layout' src/` | **NOT MET** |
| plan-13-E has landed (the seam contract exists) | `rg -n 'host_present' src/` | **NOT MET** |
| The GTK4 Debian box is reachable | `grep -n 'GTK4' .ai/remote_systems.md` → `:39`, box 2232 | **MET** |
| The flavored-import mechanism is understood | `rg -n 'musl\|glibc' src/target/linux_gtk/mod.rs` | **MET — and mandatory reading, §3.1** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> before continuing and again before deciding to stop; report every row if you stop.

## 1. Goal

- All 26 seam ops implemented on GTK4: `GtkFixed` containers + `GtkButton`/`GtkLabel`/
  `GtkEntry`; `host_measure` via `gtk_widget_measure`; `host_set_frame` via `GtkFixed`
  put/move; `host_present` via main-context iteration.
- Events wired with `g_signal_connect_data`; the command batch posted with `g_idle_add` —
  the same calls transcript mode already uses.
- The emitted `_mfb_rt_app_layout` reused **unchanged**; layout identical to macOS.
- **Every new GTK symbol declared flavor-correctly** for both AppImage worlds (§3.1).

### Non-goals (explicit constraints)

- **No solver changes.** If GTK needs one, the seam is wrong, not the solver.
- **No new seam ops.** GTK implements the contract 13-E defined; a GTK-shaped addition
  would break the three-way identity claim.
- **No GTK3 fallback.** The tree binds GTK4 (`libgtk-4.so.1`) and stays there.
- **Do not regress transcript mode** on any Linux target.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| Existing GTK app-mode code to extend | **3417 LOC** | `wc -l src/target/linux_gtk/*.rs` |
| Seam ops to implement | **26** (39 family-wide) | plan-13-E §2.1 |
| `g_idle_add` call sites already proven | **8** | `rg -c 'g_idle_add' src/target/linux_gtk/app_io.rs` |
| `g_signal_connect_data` sites already proven | **4** | `rg -c 'g_signal_connect_data' src/target/linux_gtk/bootstrap.rs` |
| Linux targets supporting app mode | **2** (aarch64, x86_64) | riscv64 explicitly excluded — `rg -n 'supports_app_mode' src/target/linux_riscv64/mod.rs` |
| libc worlds each Linux app ships for | **2** (glibc, musl) | plan-51 — one AppImage each |
| Occurrences of "AppImage" in the 2026-07-09 plan-13 docs | **0** | `rg -c AppImage planning/old-plans/superseded-plan-13-[ABC]-*.md` |

### 2.2 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| GTK signal→codegen-callback wiring is proven | **CONFIRMED** | 4 existing `g_signal_connect_data` sites |
| Main-thread marshalling via `g_idle_add` is proven | **CONFIRMED** | 8 existing sites |
| The tree binds GTK4, not GTK3 | **CONFIRMED** | `libgtk-4.so.1` in `linux_gtk/mod.rs` |
| GTK app mode is shared across Linux arches | **CONFIRMED** | one `linux_gtk` module; aarch64 and x86-64 differ only in the entry trampoline |
| riscv64 has app mode | **FALSE** | explicitly unsupported, defence-in-depth per bug-223 — an `app::` program on riscv64 must be a clean compile-time rejection |
| **`--app` emits one AppImage per libc world, with flavor-aware imports** | **CONFIRMED** | plan-51 (2026-07-18) + plan-56 (2026-07-19). **The 2026-07-09 docs know nothing about this** |
| `host_set_frame` maps cleanly onto GTK4 | **UNVERIFIED — Phase 1 spikes it** | the draft calls it "the one genuinely awkward seam call" |

## 3. Design Overview

**Where design uncertainty concentrates: `host_set_frame` on GTK4.** AppKit's `setFrame:`
is direct absolute positioning; GTK4 deliberately does not work that way. `GtkFixed`
put/move is the closest equivalent and the draft already flags it as "the one genuinely
awkward seam call". **Phase 1 prototypes it alone**, before the other 25 ops are written
against an assumption about how framing works.

If `GtkFixed` cannot express the solver's output faithfully, the three-way byte-identical
frame claim fails — and it fails on GTK, not on the solver.

### 3.1 Flavors and AppImage — what the docs predate

plan-51 (2026-07-18) made `mfb build --app` emit **one AppImage per libc world**
(`<name>-glibc.AppImage`, `<name>-musl.AppImage`), and plan-56 (2026-07-19) made the GTK
import surface **flavor-aware**. `rg -c AppImage` over the three 2026-07-09 plan-13
documents returns **zero**.

Consequence for this sub-plan: **every GTK symbol `app::` adds must be declared
flavor-correctly for both worlds.** A symbol declared for glibc only produces a musl
AppImage that fails at load — and per the plan-56 trap, a wrongly-linked musl binary can
*run fine* because musl absorbs glibc sonames, so only inspecting `DT_NEEDED` detects it.
The acceptance below requires that inspection, not a successful launch.

**Where correctness risk concentrates:** flavor declaration, for exactly that reason — the
failure is silent and a passing smoke test does not catch it.

**Rejected alternative:** *use `GtkBox`/`GtkGrid` and let GTK lay out.* Rejected: layout
would then be GTK's, not the solver's, and frames could not match macOS or the headless
host. The whole point of an emitted solver is one layout everywhere.

**Rejected alternative:** *add a GTK-specific seam op for framing.* Rejected: a
per-backend op is a divergence in the contract, and the contract is what makes three-way
identity checkable.

## Compatibility / Format Impact

- **New:** GTK4 widget creation, framing, and event wiring; new GTK symbols in both
  flavored import sets.
- **Unchanged:** the solver; the seam contract; transcript mode on every Linux target;
  the GTK4 soname.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — spike: `host_set_frame` on GTK4 (the awkward one)

- [ ] Prototype `host_set_frame` alone via `GtkFixed` put/move + allocation, driven by
      hand-supplied rects.
- [ ] Confirm it reproduces a frame set the headless host produced.

Acceptance: hand-supplied rects place widgets exactly. If `GtkFixed` cannot express them,
stop — the three-way identity claim depends on this and the seam may need renegotiating
with 13-E.
Commit: —

### Phase 2 — the widget set and events

- [ ] `GtkFixed` containers + `GtkButton`/`GtkLabel`/`GtkEntry`; `host_measure` via
      `gtk_widget_measure`; `host_present` via main-context iteration.
- [ ] Events via `g_signal_connect_data`; command batch via `g_idle_add` — the same calls
      transcript mode uses.
- [ ] Reuse `_mfb_rt_app_layout` **unchanged**.

Acceptance: the canonical program renders and is interactive on the Debian aarch64 box.
Commit: —

### Phase 3 — flavors, and the three-way identity proof (largest blast radius last)

- [ ] Declare every new GTK symbol for **both** libc worlds (§3.1).
- [ ] Build both AppImages; **inspect `DT_NEEDED` on the musl one** — do not rely on it
      launching, because a wrongly-linked musl binary runs fine (plan-56).
- [ ] Runtime: the canonical program on the Debian aarch64 box.

Acceptance: frames are **identical to macOS and to the headless host** for the same tree;
both AppImages build; the musl one's `DT_NEEDED` shows no glibc soname. A successful launch
alone does not satisfy this.
Commit: —

## Validation Plan

- Tests: frame equality against 13-D's headless goldens and 13-E's on-device frames.
- Coverage check: this sub-plan adds no shared-lowering branch, so the byte-identity gate
  is not the guard here — the guard is the three-way frame comparison.
- Runtime proof: the Debian aarch64 GTK4 box (`.ai/remote_systems.md:39`, box 2232), plus
  the `DT_NEEDED` inspection of the musl AppImage.
- Doc sync: none here.
- Acceptance: the project's full suite.

## Open Decisions

1. **`GtkFixed` vs a custom `GtkLayoutManager`.** Recommended `GtkFixed` first (Phase 1
   decides). A custom layout manager is more idiomatic GTK but adds a class to synthesize
   for a job the solver already did.
2. **Whether x86-64 needs its own entry work.** Recommended no — the existing GTK module is
   shared across Linux arches and differs only in the entry trampoline, which app mode
   already handles.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **GTK is a fan-out sibling of macOS, not its successor.** The draft
  numbered it Phase 5 after macOS's Phase 3 while stating it "reuses the emitted solver
  unchanged". Both consume 13-D; neither consumes the other.
- 2026-07-20 — **AppImage and libc flavors are entirely absent from the 2026-07-09 docs**
  (`rg -c AppImage` → 0), though plan-51 and plan-56 landed on 2026-07-18/19. Every new
  GTK symbol must be flavor-declared, and the failure is silent — a wrongly-linked musl
  binary runs fine, so acceptance requires `DT_NEEDED` inspection.
- 2026-07-20 — Backend citations in the draft's §8.0 table were substantially wrong
  (`g_application_run`, `g_idle_add`, the pipe, the GTK soname and a claimed symbol table
  all cited at lines that hold something else). Locate by symbol.

## Summary

The engineering risk is one call: `host_set_frame`. AppKit frames absolutely and GTK4
deliberately does not, so the seam's most position-dependent op is the one with the least
natural GTK equivalent. Phase 1 prototypes it alone, and if `GtkFixed` cannot express the
solver's output the three-way identity claim fails here rather than in the solver.

The quieter risk is flavor declaration, where the failure mode is silent: a musl AppImage
missing a symbol declaration still launches, because musl absorbs glibc sonames. That is
why acceptance inspects `DT_NEEDED` instead of trusting a clean run.

What is left untouched: the solver, the seam contract, transcript mode on every Linux
target, and the GTK4 soname binding.
