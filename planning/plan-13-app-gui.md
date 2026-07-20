# plan-13: the `app::` native GUI package

Last updated: 2026-07-20
Overall Effort: huge (>3d) — language amendment + package surface + emitted layout solver
+ two native backends + TextArea + Table

This plan adds `app::`, a built-in package that gives an MFBASIC program real native
widgets: a window, containers, buttons, labels, inputs, a multi-line attributed text
editor, and a widget-cell table — laid out by an emitted solver and rendered by AppKit on
macOS and GTK4 on Linux.

The single behavioral outcome of the foundation: a `mfb build -app` program that calls
`app::window`, adds a `Column` of `Button`s and `Label`s, and loops on `app::poll`
presents a real native window that lays out identically on macOS, on GTK4, and under the
headless host — and re-flows on drag-resize without the worker running.

> **`app::` the package is not app *mode*.** App mode (`mfb build -app`) already exists
> and hosts console I/O in a window (`src/target/macos_aarch64/app/`,
> `src/target/linux_gtk/`). It has **no widget concept**. `app::` is the package this
> plan adds *on top of* that infrastructure. Keep the two straight when reading anything
> dated before 2026-07-20 — several claims blur them.

References (read first):

- `src/docs/spec/language/15_resource-management.md:30` — the sentence this feature must
  amend. §Prerequisites.
- `src/target/shared/code/term_grid.rs` — **the closest precedent for the emitted
  solver**: 1202 lines, `emit_grid_alloc:289`, `emit_grid_present:853`.
- `src/target/macos_aarch64/app/` and `src/target/linux_gtk/` — the existing app-mode
  hosts the backends extend.
- `src/builtins/term.rs` (331 LOC), `net.rs` (753), `audio.rs` (757) — the existing
  builtin packages `app::` is sized against.
- `src/syntaxcheck/builtins.rs`, `src/syntaxcheck/resources.rs`, `src/ir/verify/mod.rs` —
  the three checkers the language amendment touches (§2.2).

## Prerequisites

> ### **The language amendment (13-L) is not a phase of this feature — it is its gate.**

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| The spec permits a `RES` parameter to name a resource union | `rg -n 'no generic resource supertype' src/docs/spec/language/15_resource-management.md` → still present at `:30` | **NOT MET — the spec forbids it** |
| All three checkers accept variant→union widening in a borrow parameter | `rg -n 'WIDGET_VARIANTS' src/` | **NOT MET** |
| App mode works on both target platforms | `ls src/target/macos_aarch64/app/ src/target/linux_gtk/` | **MET** |
| A GTK4 Linux box is reachable for backend proof | `grep -n 'GTK4' .ai/remote_systems.md` → `:39`, box 2232 | **MET** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every row before you continue and again before you decide to stop. Never act on a
> status you did not just verify. **If you stop, report the status of every row**, not
> only the one that blocked you.

**Row 1 is the whole feature's gate.** `15_resource-management.md:30` says, verbatim:

> *"A resource value may be passed only to a function whose parameter is declared `RES`
> and explicitly names that **concrete** resource type… There is no generic resource
> supertype, no structural matching of handles, and no implicit conversion between
> resource types."*

`app::Widget` as a parameter type is exactly what that prohibits, and `app::setVisible`,
`getSize`, `frame`, `attach`, `slot` and Table's `setWidget` all need it. So `app::`
cannot be built without a **deliberate, specified language change**. 13-L is that change,
it lands first, and everything else is gated behind it.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| `app::` implementation in the tree | **none** | `ls src/builtins/app.rs` → no such file; `rg -o '"app\.[a-z]' src/` → 0 runtime calls |
| plan-13 implementation commits | **1** (the design doc itself) | `git log --oneline --grep='plan-13' \| wc -l` |
| `app::` **types** the surface declares | **11** | `sed -n '122,403p' planning/plan-13-A-app-builtin.md \| rg -o 'app::[A-Z][A-Za-z]*' \| sort -u \| wc -l` |
| `app::` **functions** the surface declares (before overload expansion) | **32** | same, `[a-z]` |
| Existing builtin packages, for scale | `term.rs` 331 LOC, `net.rs` 753, `audio.rs` 757 | `wc -l src/builtins/{term,net,audio}.rs` |
| Builtin packages registered today | **26** | `ls src/builtins/*.rs \| wc -l` (minus `mod.rs`) |
| Closest emitted-helper precedent (`term_grid.rs`) | **1202 lines** | `wc -l src/target/shared/code/term_grid.rs` |
| The solver's own budget in the 2026-07-09 draft | "~1500–2500 lines of emitter" | plan-13-A §Phase 2 — **unmeasured; see §2.3** |
| Existing app-mode infrastructure, macOS | **4372 LOC** | `wc -l src/target/macos_aarch64/app/*.rs` (`mod` 791, `bootstrap` 978, `term_view` 1543, `app_io` 1060) |
| Existing app-mode infrastructure, Linux/GTK4 | **3417 LOC** | `wc -l src/target/linux_gtk/*.rs` (`mod` 1134, `bootstrap` 843, `term_draw` 817, `app_io` 623) |
| Targets that support app mode (and therefore `app::`) | **3 of 5** | `src/target.rs:430-437` — macos-aarch64, linux-aarch64, linux-x86_64. riscv64 is explicitly **not** (`linux_riscv64/mod.rs:44`); Windows will not be (plan-47) |
| Native→MFBASIC callback mechanisms in the tree | **0** | §2.4 |
| Ways to mint a `RES` record outside a `LINK` block | **0** | §2.4 — **new compiler capability** |

**32 functions with per-arity/per-type overload sets would make `app::` the largest
builtin package in the tree by a wide margin** — more than double `audio.rs`. That, plus
a solver larger than `term_grid.rs`, is why this cannot be three documents.

### 2.2 The three checkers the amendment touches

Argument types reach three independent checkers and all three must learn the same rule.
The 2026-07-09 draft got this right — and corrected an earlier draft that had called it
"a global fix at the `term::` seam". **The correction stands; only its line numbers have
rotted (§2.3).**

| # | Checker | Today |
|---|---|---|
| 1 | `src/syntaxcheck/builtins.rs` | one flat `param_types` list per builtin; must become a per-overload table selected by `expression_compatible()` |
| 2 | `src/builtins/app.rs::resolve_call` | **does not exist yet**; every existing package does context-free `exact()` string matching, so it cannot see that `app.Button` is a variant of `app.Widget` |
| 3 | `src/ir/verify/mod.rs::compatible` | the sole rejecter on both paths per plan-20; must accept the same widening |

### 2.3 The 2026-07-09 citations have rotted — including the ones marked "re-verified"

plan-13-A §10 opens with a note saying *"Line numbers below were re-verified on
2026-07-09."* Eleven days later, most of them are wrong, several badly:

| Claim in plan-13-A §10 | Actual today |
|---|---|
| `check_term_builtin_call` at `builtins.rs:879` | **`:426`** (off by 453) |
| `normalize_builtin_call_arguments` at `builtins.rs:1701` | **`:864`** (off by 837) |
| `ir/verify/mod.rs::compatible` at `:3411` | **`:4343`** (off by 932) |
| `compatible()` at `syntaxcheck/types.rs:145-170` | **`types.rs:117`** |
| `is_resource_type` at `syntaxcheck/types.rs:328` | **`syntaxcheck/resources.rs:4`** — *wrong file* |
| `uses_term` at `shared/code/mod.rs:590` | **`:747`** |
| `call_param_name_overloads` at `builtins/mod.rs:411-437` | **`:494`/`:507`** |

Every underlying *claim* still holds — the functions exist and do what the plan says.
Only the coordinates rotted. That is the argument for the discipline this rewrite
applies throughout: **a plan states the command, not just the line**, because a verified
line number has a half-life measured in days.

### 2.4 What app mode gives you, and the four things it does not

App mode is 7789 LOC of working infrastructure — but it is **a terminal in a window**.
Both backends render a transcript (`NSTextView` / `GtkTextView`) plus, when `term::` is
in use, a runtime-synthesized character-cell grid. Neither has any widget concept.

The threading split plan-13 assumes is real and already load-bearing: **the toolkit owns
the main thread and MFBASIC never runs on it.** macOS `_main` creates the app, spawns a
worker pthread running the program, then `[app run]` never returns; GTK does the same via
`g_application_run` and spawns the worker from `activate`. Cross-thread discipline is
strict and established — GTK writes marshal through `g_idle_add`, AppKit appends through
`performSelectorOnMainThread:waitUntilDone:`.

**There is no native→MFBASIC callback mechanism anywhere in the tree.** Every native
callback that exists (`keyDown:`, `drawRect:`, GTK `activate`/`key-pressed`) is
hand-emitted assembly that calls C functions and pokes fixed state slots; none branches
into user-authored MFBASIC. The audio callbacks are **not** a counter-example — they are
pure native producer/consumer shims that flip ring-buffer state under a mutex while
MFBASIC *polls down* via `audio::write`/`read`.

That validates the design rather than limiting it: **plan-13-A §1's "no callbacks —
retained tree, polled events" is a consequence of the seam as built, not an arbitrary
choice.** Anyone tempted to "improve" it with callbacks would be inventing a mechanism
that does not exist at any layer.

The four things `app::` must add that do not exist today:

| # | Gap | Today | Touches existing code? |
|---|---|---|---|
| a | A **widget-handle allocator + table** | 7 hardcoded singleton slots (`objc_setAssociatedObject`) on macOS; a flat `ST_*` offset block on Linux. No allocation, no id→pointer map, no lifetime tracking | yes |
| b | A path for a **codegen-emitted helper to mint a `RES` record** | plan-53's 80-byte resource record with `STATE` + a close op is exactly the right shape — but it is reachable **only** through a declared `LINK` block against a real C symbol (`ir/link.rs:363-372`, `:393-397`). And `LINK` has no function-pointer ctype (`link.rs:552`), so an MFBASIC function cannot be registered as a native handler that way either | **yes — this is new compiler capability** |
| c | An **event queue** native callbacks write and MFBASIC drains | the `keyDown:`→pipe→`io.input` path is the only worker↔main channel and carries raw bytes only | no — sits beside |
| d | A **bootstrap that can be told to skip the transcript** | the transcript view is created unconditionally; `AppEntrySpec` (`types.rs:636-641`) carries only `language_entry_accepts_args` and `uses_term` — there is no room for the decision | yes |

**(b) is the finding the 2026-07-09 design does not cover.** It assumes widget handles are
resources with close ops, which is right — but every existing resource-with-close-op in
the tree comes from a `LINK` declaration, and `app::`'s come from an emitted runtime
helper. That capability has to be built, and it belongs in 13-A.

### 2.5 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| The spec forbids resource-union parameters | **CONFIRMED** | `15_resource-management.md:30`, verbatim |
| `compatible()` already implements variant→union subsumption for *bindings* | **CONFIRMED** | `syntaxcheck/types.rs:117`; the amendment opens the parameter position only |
| No test passes a concrete resource into a resource-union *parameter* | **CONFIRMED** | `resource-union-valid` exercises `File→Stream` at a `RES` binding initializer, never at a call site |
| Three independent checkers must learn the rule, not one | **CONFIRMED** | §2.2 |
| An emitted solver of this shape has a precedent | **CONFIRMED** | `term_grid.rs`, 1202 lines — but it does *less* (no measure callback, no nested flex), so it is a floor, not an estimate |
| `app::` the package exists in any form | **FALSE** | no `builtins/app.rs`, no `app.*` runtime calls, no spec file |
| App mode provides a widget concept to build on | **FALSE** | 7789 LOC of transcript-in-a-window; no widget, layout or event surface (§2.4) |
| A native→MFBASIC callback mechanism exists to build on | **FALSE** | zero in the tree; every native callback is hand-emitted assembly. The audio callbacks are producer/consumer shims, not a callback-up precedent (§2.4) |
| A codegen-emitted helper can mint a `RES` record with a close op | **FALSE** | resource records are `LINK`-only (`ir/link.rs:363-372`). **New compiler capability required** (§2.4 gap b) |
| The toolkit owns the main thread and MFBASIC runs on a worker | **CONFIRMED** | macOS `[app run]` never returns; GTK `g_application_run`; worker spawned in both. The design's threading assumption is already load-bearing |
| `app::` can ship on every target | **FALSE** | 3 of 5 — riscv64 has no app mode (`linux_riscv64/mod.rs:44`), Windows will not (plan-47) |
| The solver is 1500–2500 lines | **UNMEASURED** | the draft's own estimate, with no derivation. `term_grid.rs` at 1202 for a simpler problem suggests it is a floor |

## 3. Design Overview

The design in the 2026-07-09 documents is **sound and is preserved**. This rewrite changes
how it is *cut*, not what it is. The locked decisions stand:

- **Shadow tree on the worker, native tree on the main thread**, reconciled at `app::sync`.
- **Layout is native-owned**: the emitted `_mfb_rt_app_layout` runs on the main thread from
  `host_present` and from the native resize handler, with no worker involvement.
- **Non-blocking `sync` + `app::poll` for pacing**; `poll` returns FALSE before that
  window's first `sync` and never parks.
- **Detach, not destroy**: `remove`/`close` detach; descendants are reparented to an
  offscreen holder before a window dies.
- **`get`/`set` naming**, uniform pairs overloaded on handle type.
- **A `RES app::Widget` argument lowers to the raw handle**, not a tagged union — the
  shadow node already carries its kind byte, so widening is representation-neutral.

### 3.1 What this rewrite changes

**(a) The language amendment is a gate, not a phase.** It was plan-13-A's "Phase 0". A
language spec change that everything depends on is a precondition; burying it as phase
zero of an x-large document means it is negotiated mid-flight.

**(b) Three documents become eleven.** A is `x-large`, B and C are `large` — all three
above the small/medium band a sub-plan must fit. A alone carries 8 phases, one of which
(the solver) is bigger than most whole sub-plans in this repo.

**(c) The fan-out is made visible.** A's phases run 0→7 linearly, which hides that
**GTK does not depend on macOS** — both consume the solver. Phase 5 (GTK) reusing "the
emitted solver unchanged" is stated in the draft itself; the numbering contradicts it.

**(d) Two units are freed to block on nothing.** `text::` AttributeString (was B Phase 1)
is pure worker-side value code with no host seam — B's own text calls it "independently
valuable" and "genuinely headless", while B's header says "Depends on plan-13-A being
landed." That header is wrong, and it is the same defect plan-47-F had.

**Where design uncertainty concentrates:** the emitted layout solver. It is the largest
single item, its size is the one unmeasured number in the plan, and it must produce
byte-identical frames across macOS, GTK4 and the headless host. Everything visual depends
on it. **13-S leads with the headless host so the solver is provable without a display**,
which is the draft's best structural idea and is preserved.

**Where correctness risk concentrates:** lifetime. Detach-not-destroy, orphan reparenting,
per-widget close ops, and union tag-dispatch drop interact, and the failure mode is a
double-free or a leak rather than a wrong pixel. It gets its own sub-plan (13-D) rather
than being a phase inside a larger one.

## Feature map

**Letters are identifiers, not an order.** Execution is topological over the graph below.
Every unit is additionally gated behind §Prerequisites.

```
  BLOCKS ON NOTHING:
    13-T  text:: AttributeString (pure value code, headless, independently valuable)

  THE GATE:
    13-L  language amendment: resource-union parameters  ──┐
                                                           │
    13-A  package skeleton, types, overload tables  ◄───────┘
      │
    13-S  emitted layout solver + headless host   ← the largest single item
      │
      ├──► 13-M  macOS/AppKit backend ──► 13-E  events, pacing, Input I/O
      ├──► 13-G  GTK4 backend
      ├──► 13-B  TextArea            (also ← 13-T)
      └──► 13-C  Table               (also ← 13-T for the addTextArea overload only)
                     │
    13-D  lifetime & detach correctness  ← 13-M + 13-G
    13-Z  polish, docs, worked examples  ← everything
```

Dependency list, in the form the executor checks:
`13-T ← nothing`; `13-L ← nothing`; `13-A ← 13-L`; `13-S ← 13-A`;
`13-M ← 13-S`; `13-G ← 13-S`; `13-E ← 13-M`; `13-B ← 13-S + 13-T`;
`13-C ← 13-S`; `13-D ← 13-M + 13-G`; `13-Z ← all`.

| Unit | Was | Effort | Produces |
|---|---|---|---|
| **13-L** | A Phase 0 | small | the spec amendment + the three checkers accepting variant→union widening in borrow position |
| **13-T** | B Phase 1 | medium | `text::AttributeString`, the span/LUT encoding, `text::setAttribute`/`getAttributes`/`&` |
| **13-A** | A Phase 1 | medium | the `app::` package: 11 types, 32 functions as overload sets, close-op registration, **and the ability for an emitted helper to mint a `RES` record outside `LINK`** (§2.4 gap b — new capability) |
| **13-S** | A Phase 2 | medium–large **(measure first)** | `_mfb_rt_app_layout` + the `headless` host + the golden layout matrix |
| **13-M** | A Phase 3 | medium | the AppKit backend and the host-protocol seam |
| **13-G** | A Phase 5 | medium | the GTK4 backend against the same seam |
| **13-E** | A Phase 4 | medium | click/double-click/close/resize events, `app::poll`, `Input` round-trip |
| **13-B** | B Phases 2–5 | medium | `app::TextArea` + the attribute serializer |
| **13-C** | C | medium | `app::Table`, the widget-cell grid, native-side virtualization |
| **13-D** | A Phase 6 | medium | detach/orphan/close-op correctness, proven leak-free |
| **13-Z** | A Phase 7 | small | the calculator example, spec + man docs |

`app::` ships on **3 of 5 targets** — macos-aarch64, linux-aarch64, linux-x86_64. riscv64
has no app mode by deliberate design (`linux_riscv64/mod.rs:44`, defence-in-depth per
bug-223) and Windows will not (plan-47). Every sub-plan's acceptance is scoped to those
three; a program calling `app::` on riscv64 must be a clean compile-time rejection, not a
broken binary.

**13-S's effort is deliberately not pinned.** It is the one number nobody measured
(§2.3), and it is the item that decides whether this feature is `huge` or worse.
**Measure it before scheduling anything after it** — see 13-S §Phase 0.

## Compatibility / Format Impact

- **Changed (13-L):** `15_resource-management.md` gains the variant→union borrow-parameter
  amendment; three checkers accept it. Widening stays directional — a union actual into a
  concrete parameter must still be rejected, and every registered close op and
  `thread::transfer`/`accept` keeps concrete-typed parameters, so no blocklist is needed.
- **New:** the `app::` package (27th builtin), `text::AttributeString`, a `headless` host
  backend, and `--app-host headless`.
- **Unchanged:** app mode's existing transcript behavior; `io::`/`term::` console lowering
  in GUI sub-mode; every other builtin package.

## Validation Plan

- Tests: per sub-plan. The layout matrix (13-S) is golden-driven through the **real
  emitted solver** under the headless host — the only way to prove layout without a display.
- Coverage check: `tests/rt-behavior/app/` will be new; confirm its goldens actually land
  in the gate's denominator. `tests/acceptance/` has **no** `golden/` dir by design — do
  not put the layout proofs there and assume coverage.
- Runtime proof: on-device on macOS, and on the Debian aarch64 GTK4 box
  (`.ai/remote_systems.md:39`, box 2232). Frames must match the headless host
  byte-for-byte on both.
- Doc sync: `15_resource-management.md` (13-L), a new `src/docs/spec/stdlib/` `app::`
  section, and man pages per `.ai/man_package_template.md`.
- Acceptance: `scripts/test-accept.sh` green.

## Open Decisions

1. **Whether 13-L belongs to plan-13 at all.** It is a *language* change that happens to
   be motivated by a GUI package. Recommended: keep it here, because nothing else in the
   language wants it and orphaning it would leave an amendment with no consumer — but
   land it as its own commit series with its own spec update, never mixed into `app::`
   package commits.
2. **13-S's real size** (§2.3). Recommended: spike the Row-only single-axis case first and
   extrapolate, before committing to the full `Direction × Justification × Align` matrix
   estimate. `term_grid.rs`'s 1202 lines for a simpler problem is the floor.
3. **Whether 13-C needs 13-T.** C's `addTextArea` table overload is the only coupling.
   Recommended: ship 13-C without that overload and add it when 13-B lands, so C does not
   wait on the attribute machinery.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **The language amendment was a phase; it is a gate.** plan-13-A Phase 0
  amends `15_resource-management.md` and three checkers, and every other unit depends on
  it. Promoted to §Prerequisites.
- 2026-07-20 — **§10's "verified" citations have rotted** (§2.3): `:879`→`:426`,
  `:1701`→`:864`, `:3411`→`:4343`, and `is_resource_type` is in `resources.rs`, not
  `types.rs`. The claims all still hold; only the coordinates moved. Every measurement in
  this rewrite therefore carries its command.
- 2026-07-20 — **plan-13-B's header dependency is wrong.** It says "Depends on plan-13-A
  … being landed"; its own Phase 1 says the `text::` AttributeString layer is
  "independently valuable" and "genuinely headless: `text::` is pure worker-side value
  code with no host seam." Split out as 13-T, blocking on nothing.
- 2026-07-20 — **GTK does not depend on macOS.** A's linear Phase 3 (macOS) → Phase 5
  (GTK) numbering hides a fan-out its own text states — Phase 5 "reuses the emitted
  solver unchanged". Both consume 13-S.
- 2026-07-20 — **All three documents were over the sub-plan band** (x-large, large,
  large). Re-cut into 11 small/medium units.
- 2026-07-20 — **plan-13-C's "soft dependency" on B is now a hard one or none.** C
  declared plan-13-B "a soft dependency (only the `addTextArea` table overload waits on
  it)". Soft dependencies are how two plans braid; C now ships without that overload and
  gains it when B lands (§Open Decisions 3).
- 2026-07-20 — **`app::` and app *mode* are different things** and pre-2026-07-20 material
  blurs them. App mode exists and hosts console I/O in a window; it has no widget concept.
  Called out at the head of this document.

## Summary

The design is sound and is preserved — the shadow tree, native-owned layout, non-blocking
`sync`, detach-not-destroy and the `get`/`set` surface are all locked and unchanged.

What was wrong was the shape. A language-spec amendment that the entire feature depends on
was filed as "Phase 0" of an x-large document; three documents each sat above the sub-plan
band; a linear phase numbering hid that the two native backends are a fan-out; and the one
unit that blocks on nothing — `text::AttributeString` — was declared to depend on
everything.

The one genuinely unmeasured number is the emitted solver's size, and it is the number
that decides this feature's cost. `term_grid.rs` does a simpler job in 1202 lines; 13-S
measures before anything is scheduled behind it.

What is left untouched: app mode's existing transcript behavior, console `io::`/`term::`
lowering, and every other builtin package.
