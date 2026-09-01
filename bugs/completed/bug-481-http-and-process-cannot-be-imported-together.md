# bug-481: `IMPORT http` and `IMPORT process` in one project fail to build — two builtins collide on the bare name `Stream`

Last updated: 2026-08-31
Effort: small as a rename; the real fix is bug-480 Phase 4 (package-scoped type names)
Severity: HIGH
Class: Correctness (language / package namespace)

Status: **FIXED** by bug-480 Phase 4b (`47dc677e5`)
Regression Test: `tests/rt-behavior/packages/http-process-coexist-rt/`

## STATUS: FIXED

Fixed exactly as this document's own "Do not take the rename" section required:
by bug-480 Phase 4b, not by renaming `process::Stream`. A builtin value type's
DECLARED identity is now package-qualified, so the two packages declare
`http.Stream` and `process.Stream` — different names — and the collision that
made the pair unbuildable cannot form.

Verified by running the Failing Reproduction below verbatim on macos-aarch64
against `target/release/mfb` at `47dc677e5`:

```
$ mfb build /tmp/b481check
Building b481check (executable) for macos-aarch64
Wrote executable to /tmp/b481check/build/b481check.out
$ /tmp/b481check/build/b481check.out
hello
[exit 0]
```

The `SYMBOL_DUPLICATE_TOP_LEVEL` on `builtins/http.mfb:93` is gone; the program
builds and runs.

**The regression test is not where this document predicted.** It is a
`rt-behavior` fixture, not `tests/syntax/packages/http-process-coexist/` — the
syntax bucket pins a compiler diagnostic, and the assertion worth keeping here is
that the program *builds and runs*, which only a runtime fixture makes.

**`Stream` is still an ambiguous LEAF**, and that is by design rather than an
oversight: the prose sweep reports it as AMBIGUOUS in five files because two
packages own the name. That is now a documentation question (which spelling a man
page shows), not a build failure — the compiler distinguishes them by package.

Importing both `http` and `process` into the same project is impossible. No use
of either package is required — the two `IMPORT` lines alone are fatal:

```
/tmp/e473b/builtins/http.mfb:93 error[2-201-0010 SYMBOL_DUPLICATE_TOP_LEVEL]: top-level symbol is declared more than once
               Top-level symbol `Stream` was already declared in builtins/process.mfb:1.
```

The diagnostic points at `builtins/http.mfb:93` — *generated* source the
developer never wrote, cannot open, and did not cause. Nothing in it names a
package the developer imported or an action they could take.

`http::Stream` is a **union** (`tcp::Socket | tls::Socket`, the transport of an
exchange). `process::Stream` is an **enum** (`StdOut`, `StdErr`). Neither
requires a package prefix today, so both land in the compiler's flat top-level
type namespace under the bare name `Stream`, and the second one to be injected
is rejected as a redeclaration.

## Do not take the rename, added 2026-08-31 (coordinator, pre-dispatch)

The header offers "small as a rename". Measured, that shortcut is a **breaking
change to shipped public API that bug-480 Phase 4 then makes unnecessary** — so
it costs users a migration and buys nothing durable.

What a rename would touch:

```
$ grep -rn 'Stream\.StdOut\|Stream\.StdErr\|process::Stream' \
    src/codegen/builtins/process/ tests/ src/docs/ | wc -l     ->  9
$ grep -rn 'http::Stream' src/ tests/ | wc -l                  -> 15
$ grep -rn '"Stream"' src/codegen/builtins/{process,http}/mod.rs
  process/mod.rs:68:  pub(crate) const STREAM_TYPE: &str = "Stream";
  http/mod.rs:424:    name: "Stream",
```

The count is small; the *kind* of breakage is not. `process::Stream` is an enum
whose members users write **unprefixed** — `process::receive(p, from:
Stream.StdOut)` is the spelling `func_receive.rs` DESC teaches. Renaming it
breaks every program and every man example that reads a child's stderr, with no
deprecation path. `http::Stream` is a union naming an exchange's transport and is
less often written by hand, so if a rename were ever forced, that is the side to
move — but it is still public surface.

**And the rename is throwaway.** This document already identifies the real fix as
bug-480 Phase 4 (package-scoped type names). Once a type resolves by
`(package, name)`, `http::Stream` and `process::Stream` coexist untouched and the
collision cannot recur for the *next* pair of packages either. A rename fixes one
pair, permanently costs a public name, and leaves the mechanism in place.

**Therefore: this bug is sequenced strictly after bug-480 Phase 4, and its fix is
to add the regression fixture, not to rename anything.** If Phase 4 slips far
enough that the collision must be unblocked sooner, raise it as a scheduling
decision for the user rather than spending a public name — and if a rename is
then chosen, move `http::Stream`, never `process::Stream`.

The one piece of work that is *not* blocked and should land regardless: the
diagnostic. It currently points at `builtins/http.mfb:93` — generated source the
developer never wrote and cannot open — and names no package they imported nor
any action they could take. That is a defect in its own right whichever way the
collision is resolved.

## Failing Reproduction

macos-aarch64, `target/release/mfb` at 744c7c175. A plain `mfb init` project.

```basic
IMPORT process
IMPORT http
IMPORT io

FUNC main AS Integer
  io::print("hello")
  RETURN 0
END FUNC
```

```
$ mfb build /tmp/e473b
Building e473b (executable) for macos-aarch64
/tmp/e473b/builtins/http.mfb:93 error[2-201-0010 SYMBOL_DUPLICATE_TOP_LEVEL]: top-level symbol is declared more than once
               Top-level symbol `Stream` was already declared in builtins/process.mfb:1.
```

Dropping either `IMPORT` builds clean.

### It is exactly this one pair

A project importing **all 27 other builtin packages at once** builds:

```
$ # src/main.mfb = one IMPORT line per package from `mfb man`, minus `process`
$ mfb build /tmp/e473c            # "mode": "app", so app + canvas are included
Wrote executable to /tmp/e473c/build/e473c.app
```

Add `IMPORT process` back and only the `Stream` error appears. So `process` is
the single builtin that cannot be combined with another, and `http` is the
single package it cannot be combined with.

The other two cross-package name collisions in the builtin surface are harmless
for the same reason this one is not — they are on **resource** types, and a
resource type already *requires* its package prefix:

| bare name | owners | kind | coexist? |
|---|---|---|---|
| `Socket` | `tcp`, `tls`, `udp` | resource | yes — prefix required, so no bare name is ever declared |
| `Listener` | `tcp`, `tls` | resource | yes — same |
| `Stream` | `http`, `process` | union / enum | **no** |

(`sed 's/.*:://' <all exported type names> | sort | uniq -d` over the 93 names
`mfb man <pkg> types` renders for the 28 builtin packages.)

`examples/network-client/src/main.mfb` imports `tcp`, `tls` and `udp` together
and uses `tcp::Socket` and `tls::Socket` in the same file (lines 108, 151) — the
proof that qualification is what makes coexistence work.

## Impact

An HTTP server that shells out — a CGI handler, a build server, a webhook runner
that invokes a script, anything gluing a request to a subprocess — cannot be
written in MFBASIC. That is a mainstream shape, and there is no workaround
inside a single project: the collision is between the two packages' *injected
sources*, not between anything the developer wrote, so no aliasing, renaming or
reordering on the developer's side avoids it.

The diagnostic makes it worse than a plain refusal. It cites a line in a file the
developer has never seen, so the reported cause is not actionable and does not
identify either `IMPORT` as the trigger.

Severity is HIGH rather than MEDIUM because it is a total loss of a common
capability, present in a shipped release, with no workaround.

## Root Cause

The builtin package sources are injected as top-level declarations into one flat
namespace, and duplicate detection is by bare name
(`SYMBOL_DUPLICATE_TOP_LEVEL`, `src/rules/table.rs`). `TypeIndex`
(`src/ir/lower.rs:4486`) likewise keys records, enums and union variants by bare
declared name with no package dimension.

Resource types escape this because they already require the `pkg::Name` spelling
(measured: bare `AS Socket` is refused with `SYMBOL_UNKNOWN_TYPE` even when only
`tcp` is imported). Records, enums and unions do not require it, so they occupy
the shared bare-name space and can collide.

This is the same root cause as bug-480 — the absent package dimension on type
names — surfacing as an outright build failure rather than a bad diagnostic.

**bug-441/plan-97 already fixed this exact class, for resources only, and did so
on a premise about the other kinds that is false.** Its rationale, still in the
source at `src/codegen/builtins/process/mod.rs:57-64`:

> The `Process` resource's **package-qualified type identity** … (bug-441 /
> plan-97: resources are addressed `process::Process`, not bare `Process`, so a
> user `TYPE Process` no longer collides).

and bug-441's own justification for doing it:

> Every other builtin surface is package-scoped: functions are `process::spawn`,
> and builtin value types (records/unions/enums) are spelled qualified in source
> (`net::Url`, `process::Process`).

That last claim does not hold. Measured at HEAD: `LET r AS Response = …` with
only `http` imported builds clean, as does `LET u AS Url = …` with `net`; and for
an enum *member* the bare `PingStatus.Ok` is the only spelling that works at all
(bug-480, Defect B). Records, unions and enums are spelled qualified **by convention and in
the man pages**, but nothing enforces it — so bug-441 hardened the one kind that
had already collided and left the others on the premise that they were already
safe. `Stream` is the first place that premise came due.

## Suggested Fix

Two options, and they are not exclusive:

1. **Immediate, small: rename one of the two.** `process::Stream` is an enum
   whose members are `StdOut`/`StdErr`; `process::StdStream` or `process::Fd`
   would say the same thing and free the name. `http::Stream` is the more
   entrenched of the two (it is the transport union threaded through the whole
   `http` surface). A rename is a breaking change to `process`'s published
   surface, so it wants a plan entry, but it unblocks the capability now.
   *This is a workaround, not a fix* — it restores one pair and leaves the next
   collision to be discovered by whoever hits it, including any user package.

2. **Real: bug-480 Phase 4 — give type names a package dimension.** Once an
   imported record/enum/union is keyed by `(package, name)` and referenced as
   `pkg::Name` from outside its package, the two `Stream`s stop colliding for
   the same reason the three `Socket`s already do not, and the whole class is
   gone — including for two *user* packages, which today have no defence at all.

Whichever lands, the diagnostic must change: a name collision between two
injected builtin sources must be reported against the developer's `IMPORT`
lines, naming both packages and the name, never against a line in
`builtins/<pkg>.mfb`.

## References

- `bugs/bug-480-package-name-resolution.md` — the same missing package
  dimension. Its Phase 4 fixes this bug, and its "Open Decisions" carries the
  interim-rename question below.
- `bugs/skipped/bug-473-qualified-enum-name-escapes-to-nir-unlocated.md` — the
  unlocated-NIR half, retired into bug-480 Defect B.
- `bugs/completed/bug-441-resources-not-package-scoped.md` +
  `planning/completed/plan-97-resources-package-scoped.md` — the same fix, applied
  to resources only. This bug is the unfinished remainder.
- `src/ir/lower.rs:4486` — `TypeIndex`, the flat bare-name type namespace.
- `src/codegen/builtins/process/mod.rs:57-64` — the `PROCESS_TYPE_ID` comment
  stating the rationale verbatim.
- `src/codegen/builtins/process/mod.rs:68,161` — `STREAM_TYPE`, the colliding enum.
- `src/codegen/builtins/http/mod.rs:423` — `add_union(RegistryUnion { name: "Stream" ... })`.
- `src/codegen/builtins/process/mod.rs` — `add_enum(RegistryEnum { name: "Stream" ... })`.
- `examples/network-client/src/main.mfb:108,151` — `tcp::Socket` and
  `tls::Socket` coexisting, because resources are qualified.
- Found during: the bug-480 prefixing-rule discussion, censusing the builtin type surface.
