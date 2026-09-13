# plan-125 — the "belongs in spec" ledger

The bridge between plan-125's two surfaces.

`mfb man` is written for the MFBASIC developer and bans compiler internals;
`mfb spec` is written for the compiler contributor and requires them. The two
standards are deliberate mirror images, and that yields the plan's most useful
property: **a sentence cut from a man page for being too internal is a
candidate spec obligation, not deleted knowledge.** It was true — it was
merely written for the wrong reader.

**Letters B–H append.** Whenever a man-surface pass cuts a sentence for
scope — §3's internals ban, §4's memory-vocabulary ban, or §1's audience
test — and the fact underneath it is real, it gets a row here. A sentence cut
for being *false* does not: that is an accuracy fix, and it belongs in the
letter's own findings ledger.

**Letters I–N consume.** This file is opened as a coverage checklist. Every
row is resolved to exactly one of:

- **COVERED** — an existing spec topic already states it. Record which topic,
  with the command that found it.
- **FILLED** — it was a genuine spec gap; record the topic and the commit that
  filled it.
- **REJECTED** — on second look the fact is not a contract (an implementation
  accident, a duplicate, or wrong). Record the disproving evidence, not a
  verdict alone.

A row left unresolved when letter N closes is a fact plan-125 deleted from one
surface without adding it to the other. That is the specific loss this ledger
exists to prevent, so N's acceptance includes "no row is unresolved".

## Ledger

| # | Man unit | Cut sentence (verbatim) | Why cut | Candidate spec package | Status | Resolution |
|---|---|---|---|---|---|---|
| 1 | `color::fromLinear` | `Together they are the seam every perceptual operation in `color` is built on, and the one the canvas software rasteriser blends through.` | `internals` | stdlib | COVERED | `src/docs/spec/stdlib/18_color.md:134` — "rasteriser blends through this same pair (`./mfb spec app canvas`" (`grep -n 'rasteriser blends through' src/docs/spec/stdlib/18_color.md`) |
| 2 | `color::fromLinear` | `The answer is found by binary search over the same 256-entry table `toLinear` reads — eight comparisons, and exactly as deterministic as a lookup. A reverse table would need 65536 entries to say the same thing.` | `internals` | stdlib | COVERED | `src/docs/spec/stdlib/18_color.md:129-130` — "The mapping is a fixed 256-entry table and a binary search over it" (`grep -n 'binary search' src/docs/spec/stdlib/18_color.md`) |
| 3 | `color::toLinear` | `That is deliberate: the software rasteriser is the oracle the GPU backends are compared against, so it must produce identical bytes on every target, and a libm transcendental does not.` | `internals` | stdlib | COVERED | `src/docs/spec/stdlib/18_color.md:261-268` — the oracle and the libm `pow` rationale (`grep -n 'oracle\|libm' src/docs/spec/stdlib/18_color.md`) |

<!-- Row format:
     # ................ sequential, never reused
     Man unit ......... `color::mix`, `tour`, `fs (overview)`, `tls types`
     Cut sentence ..... VERBATIM, in backticks. Not a paraphrase — the point of
                        the ledger is that the spec letter can judge the fact
                        for itself, and a paraphrase has already judged it.
     Why cut .......... `internals` (man §3) | `memory-vocab` (man §4) |
                        `audience` (man §1)
     Candidate pkg .... one of PACKAGE_ORDER: architecture, language, memory,
                        linker, threading, package, diagnostics, tooling,
                        package-manager, unicode, app, stdlib
     Status ........... OPEN | COVERED | FILLED | REJECTED
     Resolution ....... the topic + the command that proves it, or the
                        disproving evidence for a REJECTED row
-->

## Counters

Re-derived, never hand-maintained:

```
rows      : grep -c '^| [0-9]' planning/plan-125-belongs-in-spec.md
open      : grep -c '| OPEN |'  planning/plan-125-belongs-in-spec.md
```

| Letter | Rows appended | Running total |
|---|---|---|
| B | — | — |
| C | — | — |
| D | — | — |
| E | — | — |
| F | — | — |
| G | — | — |
| H | — | — |

## See also

- `.ai/man-content.md` §4.4 — the carve-out that sends a sentence here.
- `.ai/spec-content.md` §8 — the seam, from the spec side.
- `planning/plan-125-A-standards-tooling-pilot.md` §3.1 — why the man surface
  goes first.
