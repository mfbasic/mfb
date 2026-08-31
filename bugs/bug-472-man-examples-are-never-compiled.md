# bug-472: no `mfb man` example is ever compiled, so shipped documentation does not build

Last updated: 2026-08-30
Effort: small-to-medium (generalise an existing test; most of the mechanism is already written)
Severity: MEDIUM (shipped documentation is wrong; also hides unlocated compiler errors)
Class: Missing gate / documentation correctness

Status: Open
Regression Test: — (the gate IS the regression test; see "What a fix must produce")

## A user decision constrains this bug — read it first

`planning/completed/plan-108-A-census-standard-pilot.md`'s
Rejected-alternatives section records, as an **explicit
user decision**, that plan-108 does not build this gate:

> A permanent example-running test harness (`tests/man_examples.rs`). Rejected by
> the user: this is a docs plan and needs no test infrastructure; examples are
> verified by compiling/running them at authoring time.

**Nobody should build the harness on the strength of this bug alone.** That
decision was made for a plan whose job was prose, and it is not a standing ruling
that examples may never be gated — but it is the user's call, and the purpose of
this document is to put the evidence in front of them, not to pre-empt the
answer.

The one thing this bug does add is evidence *about the premise*. The decision
rests on "examples are verified by compiling/running them at authoring time."
Compiling them for the first time shows that this had never actually happened:
every one of the examples below **was** authored, presumably by someone who
believed it worked, and none of them compiles. That is new information the
decision was not made with — not an argument for overruling it.

## Why nothing catches it

`mfb man` pages render from `&'static str` prose fields on the registry
descriptors (`intro`/`desc`/`example` on `RegistryFunction`, `MODULE_INTRO`, …).
The compiler never reads those strings. So:

* there is no compiler error to trip — they are opaque string literals;
* `cargo test` sees nothing, because nothing renders a man page and feeds it back
  to the compiler;
* `scripts/artifact-gate.sh` is execution-free and never renders a man page
  either.

An example can therefore name a function that does not exist and ship.

This is the third independent class of content found in one evening with **no
gate at all**, and that pattern is the more valuable record than any single
instance:

| Content | Why the usual gates miss it |
|---|---|
| `tests/acceptance` | not in `cargo test`; the artifact-gate skips it for want of a `golden/` dir |
| `build.log` diagnostic goldens | `build.log` is not in `ARTIFACT_HOST_KINDS` (`ast ir hex`) — test-accept-only |
| `mfb man` examples | prose the compiler never reads; nothing renders and re-compiles it |

Each hid a real defect. The first hid a signal-death miscompile (bug-457), the
second a stale diagnostic golden (bug-465), the third everything below.

## The evidence: what compiling the examples actually found

Measured by a peer session (mfb-dc) while writing plan-108, package by package.
Every one of these **ships today in `mfb man`**:

* **`encoding::toUtf8Text` does not exist** — 7 references across `tcp`/`tls`, 3
  of them in examples. The real name is `encoding::utf8Decode`. (Instances fixed
  on main as `4416cf4e3`; the gap that let them ship is this bug.)
* **`chr(13)`** in a `tcp::write` example — `chr` is not a built-in; only a
  private `__regex_chr` helper exists.
* **`[72b, 105b]`** — there is no `b` byte-literal suffix in the language.
* **`NEXT i`** in three `general` examples — `NEXT` takes no variable.
* **12 example blocks read `.port` off a `net::Address` with no `IMPORT net`.**
* **A `net::ping` example uses `net::PingStatus.Ok`** where the accepted spelling
  is the unqualified `PingStatus.Ok`.
* **Four `thread` examples break a parameter list across lines**, which the
  grammar does not allow.
* **Several `TRAP` blocks inside a `SUB` omit their `EXIT SUB`.**

## Two of those are compiler-diagnostics bugs, not doc bugs

This is the strongest argument for the gate, because it pays off independently of
whether any prose is stale. Two of the failures above do not produce a
user-legible error at all:

* `.port` without `IMPORT net` dies as **`native plan has no storage class for
  type 'Unknown'`** — no file, no line, no rule code. That is bug-466's class.
* `net::PingStatus.Ok` dies as **`NIR local reference 'net.PingStatus' does not
  resolve`** — likewise unlocated. Filed separately as bug-473, since retired
  into `bugs/bug-480-package-name-resolution.md` (Defect B).

So a gate over the examples is also a **generator of realistic ill-formed
programs**, and it keeps finding places where the front end lets something reach
codegen and die without a location. A docs gate that doubles as a diagnostics
fuzzer is worth more than either alone.

## What a fix must produce

Every `Examples` block in every rendered `mfb man` page compiles; the ones that
are meant to run, run.

Most of the mechanism already exists and should be reused rather than rewritten:

* **`scripts/man-run-examples.sh`** (mfb-dc, on the plan-108 branch) lifts every
  Examples block out of **rendered** man output — so what is checked is what a
  developer would actually type — writes it into a scratch project, builds it,
  and optionally runs it.
* **`tests/cli_canvas_man_examples_compile.rs`** (plan-98) is the in-tree
  precedent: it already does exactly this for one package. Generalising it across
  the registry is the shape of the fix.

Two modes that a naive gate would get wrong, both learned the hard way:

* **`--test`**: `mfb build` DROPS `TESTING` blocks before codegen, so merely
  compiling a `testing` example proves nothing about it. It must be run through
  `mfb test`.
* **a `PROJECT` override**: a `thread` entry point must be an exported
  `ISOLATED FUNC` reached through an import, so it cannot live in a scratch
  `main.mfb`. Packages whose examples need a companion package need a real
  project.

A gate missing either would be *quietly vacuous* on those two packages — green,
and proving nothing.

## Timing argument

plan-108 will finish having compiled and run every example on every page, which
leaves the tree in a **known-good state that has never previously existed**. A
gate added at that moment starts green and only has to hold the line; added at
any other time it would first have to fix eight classes of breakage before it
could be switched on. "Generalise an existing test" *and* "it already passes" is
a materially easier proposition than either alone — which is worth putting to the
user alongside the decision quoted at the top.

## Update, 2026-08-31 — the whole surface has now been compiled AND run

plan-108 finished. The number this bug was written to establish is now measured
end to end, `scripts/man-run-examples.sh <pkg> --run` over every package:

**938 examples across 31 packages — 938 built, 914 run, 24 not run.** All 24 are
classified per function in plan-108-F: 11 `term` (no controlling terminal, which
`term::terminalSize`'s own page documents as raising), 5 `tls` (need a
certificate on disk and then block on `accept` for a client), and 8 `http` (all
of them bug-476, below). `app` and `canvas` run under `--app` headless.

Three things this adds to the decision the user still owns:

1. **The premise really was false, and is now true.** "Examples are verified by
   compiling/running them at authoring time" had never happened; it has now, for
   every example on every page.
2. **Running, not just compiling, is where the value was.** Compiling found the
   eight classes above. *Running* found: 16 `fs` examples that build but fail for
   any reader (they wrote under a `target/` they never created), an `os::sleep`
   example with no entry point, two `json` examples that were bare helper
   functions, a `crypto::pbkdf2` example at 600000 iterations, an `audio::play`
   example that outlasts any sane timeout — and **bug-476**, a HIGH-severity
   defect in which `http::handleRequest` accepts a request and writes no
   response at all. A compile-only gate would have missed every one.
3. **The harness a gate would reuse is now considerably more honest**, and each
   correction was itself a case of the gate agreeing with the docs instead of
   testing them: it ran examples with the *repository* as cwd (so `target/…`
   resolved to cargo's own directory and 16 broken examples passed), it had no
   per-example timeout (one `http` example stalled a run for ~3 hours looking
   like progress), its timeout implementation held the command-substitution pipe
   so every example took the full timeout, its `STDIN_FILE` plumbing was inert
   because a background job gets `/dev/null` on stdin, and it counted a
   documented non-zero exit as a failure. All fixed; see plan-108-D §Corrections
   and plan-108-F §Corrections.

So the "generalise an existing test *and* it already passes" argument at the top
is now stronger than when this was written — with the caveat that a gate would
need the 24 classified skips expressed as such, and would stay red until bug-476
is fixed.

## Blast radius

Every `mfb man` page with an example, across every built-in package. The user-
visible cost is that a developer who copies a documented example may get a
compile error — and in at least two cases an unlocated internal one with no file,
line, or rule code to act on.

References: `src/cli/man.rs` (man rendering);
`src/codegen/builtins/<pkg>/func_*.rs` (the `example` prose fields);
`tests/cli_canvas_man_examples_compile.rs` (the in-tree precedent);
`scripts/man-run-examples.sh` (plan-108 branch — the mechanism to reuse);
`planning/completed/plan-108-A-census-standard-pilot.md` Rejected alternatives
(the user decision quoted above);
`bugs/bug-466-unknown-field-type-escapes-to-codegen.md` and `bug-473` (retired
into `bugs/bug-480-package-name-resolution.md`) — the two unlocated-diagnostic
classes the examples exposed; `.ai/testing-gates.md`
(documents the other two no-gate content classes).

Credit: every example failure listed here was found and measured by a peer
session (mfb-dc) while writing plan-108; the `encoding::toUtf8Text` instances
were fixed by another (mfb-a3) as `4416cf4e3`. This document exists because
plan-108 is barred by the decision above from filing the gap it uncovered.
