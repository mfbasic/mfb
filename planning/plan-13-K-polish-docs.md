# plan-13-K: polish, docs, and the worked example

Last updated: 2026-07-20
Effort: small (<1h)
Depends on: everything (13-A, 13-B, 13-C, 13-D, 13-E, 13-F, 13-G, 13-J, 13-H, 13-I).
Feature-wide precondition: plan-13 master §Prerequisites.
Produces: the calculator example, the `app::` spec topic and man pages, and the remaining
property polish.

The last unit: live property updates, the worked example, and the documentation.

The single behavioral outcome: the calculator example builds and runs on both backends,
and `mfb man app <name>` renders for every function in the surface.

References (read first):

- `.ai/man_template.md`, `.ai/man_type_template.md`, `.ai/man_package_template.md` — the
  DOC authoring rules.
- `scripts/update_man.sh`.
- `planning/old-plans/superseded-plan-13-A-app-builtin.md` §13 — the calculator worked example this ships.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| Every other plan-13 unit has landed and been archived | `ls planning/plan-13-*` → only this file and the master | **NOT MET** |
| Both backends run the canonical program | 13-E and 13-F acceptance | **NOT MET** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> before continuing and again before deciding to stop; report every row if you stop.

## 1. Goal

- `setVisible` reflow (`display:none`), `size`/`resizable`/`title` live updates, `<0`
  fill/clamp semantics for `Size`/`Spacing`, and the `app::frame` mirror — the properties
  deferred from the backend units.
- The §13 calculator worked example, building and running on both backends.
- Spec and man docs for the whole `app::` surface, in the **right place**.
- Acceptance green.

### Non-goals (explicit constraints)

- **No new widgets, no new seam ops.** If this unit needs one, an earlier unit is
  incomplete — fix it there.
- **No behavior changes to land docs.** Documentation describes what shipped; a doc that
  needs a code change is reporting a bug in an earlier unit.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| `app::` callables to document | **79** family-wide (base 32, TextArea 25, Table 22) | master §2.1 |
| Spec topics under `src/docs/spec/stdlib/` today | **15** | `ls src/docs/spec/stdlib/ \| wc -l` |
| Man package dirs today | 24 | `ls src/docs/man/builtins/ \| wc -l` |
| Backends the example must run on | 2 | macOS, GTK4 |

### 2.2 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| Package surfaces are documented in `stdlib/` + `man/builtins/` | **CONFIRMED** | 15 stdlib topics, 24 man packages |
| `src/docs/spec/package/` is the **binary container format** | **CONFIRMED** | `01_container-format.md`, `02_binary-representation.md`, … — **the 2026-07-09 docs sent `app::` documentation here in five places** |
| Every `app::` function has a man page | **UNVERIFIED — an acceptance criterion** | 79 callables; check by enumeration, not by spot check |

## 3. Design Overview

Nothing to design. This unit exists so that the property polish and the documentation are
*someone's* explicit responsibility rather than trailing tasks on units that considered
themselves finished.

**Where correctness risk concentrates:** the documentation destination. Five tasks across
the 2026-07-09 plan-13-H and -C send `app::` documentation to `mfb spec package`. That
directory is the `.mfp` binary container format specification. Following those tasks would
put a GUI package's surface documentation inside the file-format spec.

**Rejected alternative:** *fold this into the backend units.* Rejected: it is exactly the
work that gets dropped when a unit's interesting part is done, which is why it is its own
unit with its own acceptance.

## Compatibility / Format Impact

- **New:** an `app::` spec topic; `src/docs/man/builtins/app/` pages; the calculator
  example.
- **Unchanged:** everything else — this unit ships no behavior beyond the deferred property
  updates.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — the deferred properties

- [ ] `setVisible` reflow (`display:none` skipping, already in the solver — wire the
      property).
- [ ] `size`/`resizable`/`title` live updates; `<0` fill/clamp semantics for
      `Size`/`Spacing`; the `app::frame` mirror.
- [ ] Tests: hidden-sibling reflow against the headless goldens; a live title change.

Acceptance: hiding a sibling reflows to the frames the headless host predicts; title and
size updates take effect without a rebuild.
Commit: —

### Phase 2 — docs, in the right place

- [ ] A new `src/docs/spec/stdlib/` topic for `app::`.
- [ ] `src/docs/man/builtins/app/` pages per `.ai/man_package_template.md`,
      `.ai/man_template.md` and `.ai/man_type_template.md`; run `scripts/update_man.sh`.
- [ ] **Enumerate all 79 callables and confirm each has a page** — do not spot-check.
- [ ] Document the 3-of-5 target support explicitly: `app::` on riscv64 or Windows is a
      compile-time rejection, not a runtime failure.

Acceptance: `mfb man app <name>` renders for **every** callable in the surface, verified by
enumeration; the spec topic exists under `stdlib/`, not under `package/`.
Commit: —

### Phase 3 — the worked example and the gate

- [ ] Ship the §13 calculator example.
- [ ] Runtime: it builds and runs on macOS and on the Debian aarch64 GTK4 box.
- [ ] `scripts/test-accept.sh` green.

Acceptance: the calculator runs on both backends and the acceptance suite is green. Archive
every plan-13 document to `planning/old-plans/` in the commit that lands this.
Commit: —

## Validation Plan

- Tests: the acceptance suite, plus the property tests in Phase 1.
- Coverage check: confirm the `app::` fixtures added across the family are in the gate's
  denominator — this is the last chance to notice a directory whose goldens never landed.
- Runtime proof: the calculator on both backends.
- Doc sync: this unit *is* the doc sync.
- Acceptance: `scripts/test-accept.sh` green.

## Open Decisions

1. **Whether the calculator is the right example.** Recommended keep it — the 2026-07-09
   draft calls it "a (crappy) calculator", which is honest and correct: it exercises
   buttons, labels, a grid-ish layout and live updates without pretending to be a product.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **Documentation destination corrected.** The 2026-07-09 plan-13-H and -C
  sent `app::` documentation to `mfb spec package` in five places; that is the `.mfp`
  binary container format. Package surfaces belong in `src/docs/spec/stdlib/` and
  `src/docs/man/builtins/` (master §2.5).
- 2026-07-20 — **Split out as its own unit.** Docs and deferred properties were the tail of
  plan-13-C's Phase 7; giving them their own acceptance is what stops them being dropped
  when the interesting work is done.

## Summary

There is no engineering risk here and no design. The value of making it a unit is that the
documentation and the deferred properties have an owner and an acceptance criterion, rather
than being the part of an x-large plan that everyone considers optional once the window
opens.

The one thing to get right is where the documentation goes: five tasks in the 2026-07-09
documents point at the binary container format spec.
