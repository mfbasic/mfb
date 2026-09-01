# bug-483: every helper-built `net::Address` has an unreadable `host` — the two spellings of a builtin value type stopped agreeing

Last updated: 2026-08-31
Effort: small to fix (one predicate seam), medium to find — the reported symptom
named the wrong subsystem, and the regression window was 91 commits, not the six
the first pass estimated
Severity: HIGH
Class: Correctness (regression — working calls now crash the process, and two
compile-time guarantees silently stopped being enforced)

Status: **FIXED** — see the STATUS block below.

## STATUS: FIXED (fdac03c3c)

The report was accurate about the crash and wrong about the cause. It is not a
TLS bug, not a listener bug, and not in the `tls` package at all: `tcp`, `udp`,
`net` and `audio` were equally broken, and the fix is one predicate.

**Root cause.** `363b85696` ("bug-480 Phase 4b: require the package prefix on an
imported value type") package-qualified a builtin value type's declared identity,
but only where the registry rewrites it.
`Registry::qualify_value_type_references` rewrites **signature** types, so a
member's return arrives as `net.Address`; it deliberately leaves record **field**
types bare so the injected companion source stays parseable, so `udp::Datagram`'s
`from` field is still `Address`. Both spellings therefore reach every nominal
question asked about that record — and four predicates were asking with one
spelling each. A nominal miss is not a compile error, so all four failed
silently.

| | what it decided | what the miss did |
|---|---|---|
| `builder_collection_layout::is_pointer_string_record` | record LAYOUT | the three helper-built records moved to the inlined-`String` layout while their runtime helpers kept writing absolute pointers — readers dereferenced a pointer as a block-relative offset. **This is the reported SIGSEGV.** |
| `read_only_record_type` (both copies: `ir::verify`, `ir::shape`) + `term::is_read_only_record` | a compile-time refusal | `net::Address["1.2.3.4", 80]` and `WITH addr { port := 99 }` compiled and ran. The baseline refuses both. |
| the Address-overload dispatch in `builder_values` (`tcp.connect`, `tls.connect`, `net.ping`) | which code form lowers | `tcp::connect(bound, 5000)` lowered through the `host, port` form and read the record as a `String` block: SIGSEGV. |
| `binary_repr::sections::type_id` (`TermColor`/`TermSize`) | the wire type id | fell through to the opaque zero-field fallback, so an exported `term::TermColor` encoded a record with none of its `r`/`g`/`b`. |

**The fix** is one seam rather than four literal lists:
`ParameterType::is_builtin_named(package, leaf)` (`src/types.rs`) accepts either
spelling and refuses another package's same-named leaf, so it cannot reintroduce
the flat namespace bug-481 removed. All four sites ask through it.

### Measured blast radius

Everything below crashed on `213803f96` and is correct after the fix, measured by
running one program per line, not inferred:

| call | before | after |
|---|---|---|
| `net::lookup(...)` then `a.host` | SIGSEGV | `127.0.0.1` |
| `tcp::localAddress(listener).host` | SIGSEGV | `127.0.0.1` |
| `udp::localAddress(sock).host` | SIGSEGV | `127.0.0.1` |
| `udp::receive(...)` then `dg.from.host` | SIGSEGV | `127.0.0.1` |
| `audio::devices()` then `d.name` | SIGSEGV | the three real device names |
| `tls::localAddress(listener).host` (the report) | SIGSEGV | `127.0.0.1` |
| `tcp::connect(address, timeoutMs)` | SIGSEGV | connects |
| `net::Address[...]` / `WITH addr {...}` | compiled and ran | refused, as on the baseline |

### Corrections to this document as originally filed

* **"the regression window is six commits wide".** `git log f0e4d3ff2..8b27c8a11`
  lists **91** commits. The window was bounded correctly; its size was not.
* **"the failing case is the Listener overload specifically".** The plaintext
  `tcp::localAddress(listener)` fails identically, with no TLS anywhere. That
  single measurement is what moved the search out of `tls/` — the two `tls`
  emitters named under "Where to look first" are correct and were never touched.
* **"`4cce58103` is not the culprit: its parent already crashes".** True, and it
  is also the commit that accidentally *fixed* `udp` for a while, by qualifying
  `udp::address()` — which is why `udp`'s own acceptance fixture kept passing and
  narrowed nothing.
* **"Ruled out: the declared type spelling ... qualifying all three tls sites
  does not fix the crash".** Correct as far as it went, and it is what made the
  spelling look innocent. The spelling *was* the cause — but at the four
  predicates that consume it, not at the descriptors that declare it.
* **"no new fixture is needed".** The two `rt_tls_*` tests do turn green, but
  they cover one route to one predicate. Four new gates were added instead (see
  below), because a fix that repaired only the direct TLS return would have left
  the list, the nested-record and the overload-dispatch routes broken and every
  named test green.

### The goldens were snapshots of the miscompile

Two whole tiers of committed golden encoded the broken behaviour, because the
bug-480 series regenerated them *after* the regression landed and before anyone
ran the affected programs. Both were regenerated here, and neither is a
re-baseline of a disproved test — they are drift sentinels that had drifted onto
a bug (AGENTS.md: byte-identity goldens are sentinels, not behaviour).

* **30 `.ncodesum`s**, across `audio`, `http`, `net`, `tcp`, `tls`, `udp` x 5
  targets — exactly the packages that produce or consume the three records.
  `regen-ncodesum.sh` refreshed all 132; exactly those 30 changed content, one
  for one with what the gate reported. Every other byte-identity fixture passed
  unchanged, which is what rules out a pre-existing diff riding along.

* **33 `rt-behavior` `build.log`s that pinned the crash itself.** Found with

      grep -rln '^\[exit 13[89]\]' tests/rt-behavior/

  A behaviour fixture should never legitimately end in a signal, so an
  `[exit 138]`/`[exit 139]` in one of these is *always* a dead fixture. This is
  the runtime twin of "a golden pinning a build failure is a dead fixture": the
  harness compares one crash to another and can report PASS.

  They are **flaky-by-crash**, which is why they went unnoticed and why a single
  acceptance run under-reports them. The same bad pointer lands as SIGBUS (138)
  or SIGSEGV (139) depending on where the arithmetic points, and a run may die
  earlier than the golden did and lose trailing output. One acceptance run
  surfaced 9 of the 33; the grep above finds all of them deterministically,
  regardless of which way the crash fell that day.

  `func_udp_endpoints_valid`'s committed golden is the clearest example — it
  recorded `host=` (empty) followed by `[exit 138]`, while its own remaining
  lines expect the program to reach `closed`.

  The gate applied after regenerating: **that grep must return nothing.** A
  fixture still ending in a signal would be a second defect to chase, not
  something to re-pin.

### Regression tests

| gate | what it holds |
|---|---|
| `tests/rt_net_address_record_layout.rs` | runs a program reading `host` back through four routes to the shared predicate: a bare returned record, a `List OF` it, another package's member, and a nested `Datagram.from`. RED with SIGSEGV before. |
| `tests/rt_net_address_overload_form.rs` | runs `tcp::connect(address, timeoutMs)` — the form arity cannot select. |
| `pointer_string_record_tests` (3 tests) | both spellings agree; the names are the ones the registry actually declares; **no other** declared record is on the list. Walks the registry, so it cannot drift. |
| `read_only_records_are_refused_under_either_spelling` | the guard, under both spellings, for all four compiler-owned records. |
| `tests/syntax/net/net_address_read_only_invalid/` | the two user-visible diagnostics. Nothing pinned them before — which is exactly why the guard could die unnoticed. |
| `is_builtin_named_accepts_both_spellings` | the new seam, including the prefix near-misses (`subnet.Address`, `netAddress`, `Addressed`) and another package's leaf. |
| `wire_type_ids_are_unchanged_by_the_typed_encoder` (extended) | `term.TermColor`/`term.TermSize` keep their reserved high-band ids. |

`DatagramText` was dropped from the pointer-string list: it is no longer a
declared type anywhere (`udp/mod.rs` asserts it must not be), so that arm was
dead. The new registry-walking test would have caught it.

## Original report

The rest of this document is the report as filed, preserved unedited apart from
the corrections above.

## What happens

A program that asks a TLS **listener** for its bound address, and then reads a field of
the returned `net::Address`, dies with `SIGSEGV` (exit 139).

```basic
IMPORT io
IMPORT net
IMPORT tls

FUNC main AS Integer
  RES server = tls::listen("127.0.0.1", 0, "/tmp/p98-tls/cert.pem", "/tmp/p98-tls/key.pem")
  LET b = tls::localAddress(server)
  io::print("host=" & b.host & " port=" & toString(b.port))
  RETURN 0
END FUNC
```

```
$ ./build/tlsprobe.out
[exit 139]
```

Nothing is printed — the crash is in the `&` concatenation that reads `b.host`, before
`io::print` is reached.

**It is the field read, not the call.** With the print removed the program runs past
`localAddress` and prints a following statement, then still faults on the way out:

```basic
  RES server = tls::listen(...)
  io::print("listening")        -> printed
  LET b = tls::localAddress(server)
  io::print("got address")      -> printed
  tls::close(server)
  RETURN 0                      -> [exit 139]
```

So the record comes back, and its `host` String payload is bad. `tls::listen` +
`tls::close` with no `localAddress` between them exits 0.

## It is a regression, and the window is bounded

Measured with the `git archive` + `cargo build --release` attribution technique
(`.ai/` / bug-478's notes), running the SAME four-line program against each binary:

| commit | date | result |
|---|---|---|
| `f0e4d3ff2` plan-108-C: compile every network-family example | 2026-08-30 | **`host=127.0.0.1 port=60594`, exit 0** |
| `8b27c8a11` bug-480 Phase 4b: regenerate the goldens the declared-name change shifts | 2026-08-31 | exit 139 |
| `4cce58103` bug-480 Phase 4b: qualify the remaining registry type identities | 2026-08-31 | exit 139 |
| `HEAD` | 2026-08-31 | exit 139 |

So the break is inside `f0e4d3ff2..8b27c8a11` — the bug-480 Phase 4 series and the
plan-108-C/D commits interleaved with it. `4cce58103` is *not* the culprit: its parent
already crashes.

## Ruled out

* **The declared type spelling.** `tls::localAddress`, `tls::remoteAddress` and
  `tls::connect` still name the bare `net::ADDRESS_TYPE` where `udp::address()` was
  moved to `ADDRESS_TYPE_ID` by `4cce58103`. Qualifying all three tls sites and
  rebuilding does **not** fix the crash (measured).
* **`tcp::address()`.** Reverting it to the bare name and rebuilding does not fix the
  crash either (measured). That change is correct on its own grounds — it restores the
  tcp/udp consistency `tcp_endpoints_use_the_shared_net_address_record` asserts — and is
  unrelated to this.
* **Load / contention.** bug-465's deadlines were widened for exactly that reason, but
  this reproduces on a quiet machine, in isolation, every run.
* **The cert.** A freshly generated `openssl req -x509 -newkey rsa:2048 -nodes` pair, the
  same shape the test uses.

## Where to look first

`tls::localAddress` has **two** overloads — Socket and Listener — and bug-465 finding 3
was precisely that the Listener one was missing. The Socket path is what
`tls::remoteAddress` and the tcp equivalents exercise, and those are not obviously
broken; the failing case is the Listener overload specifically. So the suspicion is that
the overload's *return* record is built through a path that the value-type namespace
change re-pointed, leaving the `host` String's payload pointer wrong — the same class of
symptom as a stale record layout, not a null handle.

`.ai/net-tls.md` and plan-110-D record that the endpoint queries reuse the `net` address
emitter and that the TLS record keeps the fd in the canonical handle slot; that shared
emitter is the first thing to read.

## Found while

Landing plan-98-F Phase 3. The two RED tests turned up in the full-suite run at the end
of that work and were **not** caused by it — plan-98's changes are Windows-only codegen
plus test expectations, and both were ruled out by measurement above. An earlier
full-suite run in the same session appeared not to show them, but that output had been
truncated by a `head -12` on exactly twelve matching lines, so it is no evidence that
they were ever green after the bug-480 series landed.
