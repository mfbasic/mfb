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
| 4 | `mfb man types list` | `A list stores its items in one contiguous allocation — a header, a lookup table that holds list order, and a packed data region — so a list value is a single owned block. Primitive items are stored as payload bytes; `String` items are stored as their UTF-8 bytes; and records, data-only unions, and *flat* nested collections (a `List`/`Map` whose own payloads are flat) are inlined into the data region as their full block. The only payloads stored as an 8-byte pointer handle rather than inline are a resource and a non-flat nested collection (one whose own payloads include a resource).` | `internals` | memory | OPEN | — |
| 5 | `mfb man types list` | `Copying a list is shrink-to-fit: the copy is re-tightened so its capacity equals its length, so over that tight prefix it is a single contiguous memory copy and no mutable working headroom leaks into the snapshot.` | `internals` | memory | OPEN | — |
| 6 | `mfb man types list` | `For a uniquely-owned `MUT` list binding written with the `name = collections::set(name, …)` idiom, the compiler may update the live buffer in place — an append into spare headroom is amortized O(1) — while a `LET` list binding remains an immutable snapshot and helper calls produce a new value. Passing or returning a `MUT` list across a function boundary freezes it into an immutable owned value, so no caller and callee ever share a mutable buffer.` | `internals` | memory | OPEN | — |
| 7 | `mfb man types map` | `A map stores its keys and values in one contiguous allocation — a header, an insertion-ordered lookup table, a packed data region, and a derived hash index. Primitive keys and values are stored as payload bytes; `String` payloads are stored as their UTF-8 bytes; and records, data-only unions, and *flat* nested collections are inlined into the data region as their full block. The only payloads stored as an 8-byte pointer handle are a resource and a non-flat nested collection. Key lookup uses an O(1)-average FNV-1a hash index that is rebuilt lazily on first use.` | `internals` | memory | OPEN | — |
| 8 | `mfb man types map` | `Copying a map is shrink-to-fit — the copy is re-tightened to its live size, so over that prefix it is a single contiguous memory copy.` | `internals` | memory | OPEN | — |
| 9 | `mfb man types map` | `For a uniquely-owned `MUT` map binding the compiler may update the live buffer in place — inserting a new key into spare headroom, or overwriting a same-size value — while a `LET` map binding remains an immutable snapshot and helper calls produce a new value.` | `internals` | memory | OPEN | — |
| 10 | `mfb man types set` | `A set is stored as a `Map`-shaped block — a header, an insertion-ordered lookup table, a packed data region, and a derived hash index — but it is a hash-indexed set of elements with no values: each element is stored as an entry key, and the per-entry value is a single implementation-detail tag byte. Membership is an O(1)-average FNV-1a hash probe for the probe-eligible element types — `Integer`, `Float`, `Fixed`, `Byte`, `Boolean`, and `String` — and a linear scan over the live entries for any other element type. The hash index is rebuilt lazily on first use, and the bucket region is shared with the `Map` layout.` | `internals` | memory | OPEN | — |
| 11 | `mfb man types set` | `Copying a set is shrink-to-fit — the copy is re-tightened to its live size, so over that prefix it is a single contiguous memory copy.` | `internals` | memory | OPEN | — |
| 12 | `mfb man types string` | `It is register-carried like `Byte`, never heap-allocated.` | `internals` | memory | OPEN | — |
| 13 | `mfb man types string` | `A `String` is heap-allocated (a length-prefixed UTF-8 buffer); a `Scalar` is a 4-byte register value (see `mfb spec memory scalar-storage`). A `Scalar` collection element occupies a 4-byte, 4-aligned inline payload.` | `internals` | memory | OPEN | — |
| 14 | `mfb man link` | `The resolver collects `LINK` aliases before ordinary top-level symbols so resource `CLOSE BY` declarations and transparent re-export aliases can forward-reference native functions.` | `internals` | language | OPEN | — |
| 15 | `mfb man link` | `The backend emits `_mfb_linker_init`, which opens each distinct declared library and resolves each declared `SYMBOL` and `FREE` deallocator into a global pointer slot. Calls to native wrappers go through generated marshaling thunks named from the `LINK` alias and function name.` | `internals` | linker | OPEN | — |
| 16 | `mfb man types` | `Compiler-owned templates such as `List`, `Map`, `Set`, `MapEntry`, `Pair`, `Partition`, `Thread`, and `ThreadWorker` are monomorphized before code is generated, so each concrete use has a fully known type.` | `internals` | language | OPEN | — |
| 17 | `mfb man lambda` | `Such a lambda may borrow an outer `MUT` binding and mutate it: the binding is loaned to the callback for the duration of the synchronous call — a borrow of the live binding, not a copy — and is the outer binding's again once the call returns.` | `memory-vocab` | language | OPEN | — |
| 18 | `mfb man lambda` | `This is an internal call-bound borrow, not a general source-level capability: non-escaping closures are not part of the v1 source language, so there are no `NONESCAPING`, `BORROW`, or lifetime annotations.` | `internals` | language | OPEN | — |
| 19 | `mfb man optimizations` | `*Stage* says where the pass runs: `NIR` (the structured native IR, before storage planning), `MIR` (the selected machine-neutral stream, before register allocation), or `machine` (after register allocation, on physical registers).` | `internals` | architecture | OPEN | — |
| 20 | `canvas::destroyFont` | `A scene carries the id, not the font, so it cannot dangle;` | `internals` | app | OPEN | — |
| 21 | `collections::distinct` | `Building the result needs memory, but running out of it is not a trappable domain error, and the `append` it uses is classified infallible for exactly that reason.` | `internals` | stdlib | OPEN | — |
| 22 | `collections::distinct` | `A call whose element type is not comparable is rejected at compile time with `TYPE_REQUIRES_COMPARABLE`, reported against the internal `collections.contains` call.` (the diagnostic's location clause) | `internals` | diagnostics | OPEN | — |
| 23 | `collections::mid` | `mid` copies the selected run using a fast contiguous path when the source entries covering the slice are stored in order and packed tightly, and falls back to a per-entry copy otherwise. A list whose entry records have been permuted without moving the underlying data — the result of a sorted directory listing, for instance — takes the fallback. | `internals` | memory | OPEN | — |
| 24 | `collections::values` | `The projection walks the lookup-entry array directly, and that array is maintained in insertion order; the hash bucket index is separate derived metadata that does not reorder it.` and `are the same traversal over the same entries and differ only in which payload field of each entry they copy` | `internals` | memory | OPEN | — |
| 25 | `collections::contains` | `on a `Set` membership is an O(1)-average hash probe for a probe-eligible element type and a linear scan otherwise.` / `An empty list always yields `FALSE`, since the loop exits on the first bounds check.` | `internals` | memory | OPEN | — |
| 26 | `fs::writeText` ×8 write functions | `The text payload is written directly from the `String`'s packed byte data.` / `The byte payload is written directly from the byte list's packed data region.` | `internals` | memory | OPEN | — |
| 27 | `canvas (overview)` | `destroying one that a presented scene still draws is safe, because the scene holds only its id.` (the reason clause) | `internals` | app | OPEN | — |
| 28 | `datetime::addDays` | `It converts `dt`'s calendar date to a serial day count, adds `days`, converts that count back to a year-month-day date, and rebuilds the `datetime::DateTime` from the new date, `dt`'s original wall-clock time, and `dt`'s original zone.` | `internals` | stdlib | OPEN | — |
| 29 | `datetime::addMonths` | `It collapses `dt`'s year and month into a single month index (`year * 12 + month - 1`), adds `months`, and splits the sum back into a target year and month with a flooring divide so that crossing year boundaries in either direction is handled correctly.` | `internals` | stdlib | OPEN | — |
| 30 | `datetime::civil` | `It probes the zone's offset one day before and one day after the named local time to bracket any single nearby transition. If both probes agree, that offset is used directly.` | `internals` | stdlib | OPEN | — |
| 31 | `datetime::compare`, `equals`, `isBefore`, `isAfter` | `it performs only signed comparisons (no arithmetic), so it cannot overflow or trap.` | `internals` | stdlib | OPEN | — |
| 32 | `datetime::dayOfYear` | `The day-of-year is computed on the proleptic-Gregorian calendar by taking the days-from-civil count of `dt`'s date, subtracting the days-from-civil count of January 1 of the same year, and adding one (`here - start + 1`), so leap years correctly extend the count past February.` | `internals` | stdlib | OPEN | — |
| 33 | `datetime::dayOfYear`, `weekday` | `no `datetime::Instant` is resolved and no zone table is consulted.` | `internals` | stdlib | OPEN | — |
| 34 | `datetime::fromMillis` | `The implementation first computes the toward-zero quotient `millis / 1000` and remainder `millis MOD 1000`; when that remainder is negative it adds `1000` to the remainder and subtracts `1` from the quotient, borrowing one second.` | `internals` | stdlib | OPEN | — |
| 35 | `datetime::inZone`, `withZone`, `offsetAt` | `for a local zone it reads the host's time-zone configuration through the `datetime::localOffset` OS intrinsic to resolve the offset.` | `internals` | stdlib | OPEN | — |
| 36 | `datetime::localOffset` | ``localOffset` is the low-level intrinsic that backs `datetime::offsetAt` for local zones and `datetime::toLocal`` | `internals` | stdlib | OPEN | — |
| 37 | `datetime::monotonicNanos` | `It is the low-level OS-seam intrinsic that backs `datetime::monotonic`` | `internals` | stdlib | OPEN | — |
| 38 | `datetime::offsetAt` | `(`datetime::ZoneKind::Local`, built with `datetime::local`, internally zone kind `2`)` | `internals` | stdlib | OPEN | — |
| 39 | `datetime::offsetAt` | `the function returns the zone's stored constant offset directly and does not consult `at` — the UTC zone stores zero, and a fixed zone stores its single configured offset.` | `internals` | stdlib | OPEN | — |
| 40 | `datetime::resolve` | ``resolve` first converts the civil date (`dt.date.year`, `dt.date.month`, `dt.date.day`) to a day count with the proleptic Gregorian calendar, multiplies by `86400` to get seconds, and adds the time-of-day contribution (`dt.time.hour * 3600 + dt.time.minute * 60 + dt.time.second`). That sum is the local second count: the seconds-since-epoch the wall-clock fields would name if they were UTC. It then subtracts `dt.offset` — the resolved UTC offset in seconds carried on the `datetime::DateTime` — to shift the local count back onto the UTC timeline` | `internals` | stdlib | OPEN | — |
| 41 | `datetime::toLocal` | `adds that offset in seconds to the instant's seconds-since-epoch to obtain a local second count, floor-divides that into whole days and the second-of-day, converts the day count to a civil year/month/day with the proleptic Gregorian calendar, and decomposes the second-of-day into hour, minute, and second.` | `internals` | stdlib | OPEN | — |
| 42 | `datetime::utc` | `(the first `datetime::ZoneKind` variant, tag `0`)` | `internals` | stdlib | OPEN | — |
| 43 | `datetime::weekday` | `The day count for that civil date is computed on the proleptic-Gregorian calendar and reduced modulo seven against a fixed reference (`floorMod(days + 3, 7)`)` | `internals` | stdlib | OPEN | — |
| 44 | `encoding::utf8Decode` | `The overload is settled once the argument type is known, so the selection is a compile-time decision, not a runtime dispatch.` | `internals` | language | OPEN | — |
| 45 | `encoding::punycodeEncode` | `The input `String` is decoded to Unicode scalar values through the package's UTF-8 decoder before encoding.` | `internals` | stdlib | OPEN | — |
| 46 | `collections (overview)` | `and access reads without copying the collection.` | `internals` | memory | OPEN | — |
| 47 | `collections::any`, `all` | `the callback position proven non-escaping is `collections::forEach`, not `any`.` (same sentence on `all`) | `internals` | language | OPEN | — |

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
| C | 20 | 47 |
| D | — | — |
| E | — | — |
| F | — | — |
| G | — | — |
| H | — | — |

## See also

- `.ai/man-content.md` §4.4 — the carve-out that sends a sentence here.
- `.ai/spec-content.md` §8 — the seam, from the spec side.
- `planning/completed/plan-125-A-standards-tooling-pilot.md` §3.1 — why the man surface
  goes first.
