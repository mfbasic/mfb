# bug-373: a user `RESOURCE` named after a built-in resource fails with an internal compiler error

Last updated: 2026-07-20
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Footgun

Status: Fixed (2026-07-21)
Regression Test: `tests/syntax/resources/resource-shadows-builtin-invalid`

Declaring a user resource whose name collides with a built-in resource type —
`RESOURCE File`, `RESOURCE Socket`, `RESOURCE Listener`, … — does not produce a
diagnostic. It produces a raw internal error from the NIR validator naming a
runtime helper the user never mentioned:

```
error: NIR declares unused runtime helper 'fs'
```

There is no rule code, no source location, no span, and nothing connecting the
message to the `RESOURCE` line that caused it. A user binding a C library with a
type they reasonably want to call `File` gets an error about the *standard
library's filesystem helper*.

**The single correct behavior a fix produces:** the collision is rejected at
check time with a proper diagnostic that names the colliding type and its
built-in origin, carries a rule code, and points at the `RESOURCE` declaration.
(Making user resources actually shadow built-ins is a language-design change and
is explicitly *not* what this bug asks for — see Non-goals.)

Found while executing plan-59-A Phase 3, as the adversarial probe for
"is `has_io_buffers` false for every native `LINK` resource?". It is — precisely
*because* this bug makes the colliding program unbuildable. That is a real
guarantee resting on an error message, which is why it is worth filing rather
than leaving as folklore.

References:

- `src/target/shared/validate.rs:107` — the internal error that surfaces
- `src/builtins/resource.rs:116-220` — `BUILTIN_RESOURCES`, the colliding namespace
- `src/builtins/resource.rs:258` — `is_builtin_resource`, the predicate a fix uses
- `planning/plan-59-A-universal-resource-record.md` — Corrections C8, where this
  was found and where the `has_io_buffers` guarantee is recorded

## Failing Reproduction

Any project with a `LINK` block binding a native resource named `File`:

```basic
IMPORT io

RESOURCE File CLOSE BY sql::close

LINK "sqlite3" AS sql
  FUNC open(path AS String) AS RES File
    SYMBOL "sqlite3_open"
    ABI (path CString, db OUT CPtr) AS status CInt32
    RETURN db
    SUCCESS_ON status = 0
  END FUNC
  FUNC close(RES db AS File) AS Nothing
    SYMBOL "sqlite3_close"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main() AS Integer
  RES db AS File = sql::open(":memory:")
  RETURN 0
END FUNC
```

```
$ mfb build
Building p59file (executable) for macos-aarch64
error: NIR declares unused runtime helper 'fs'
```

- **Observed:** `error: NIR declares unused runtime helper 'fs'`, exit 1, no
  rule code, no span, no mention of `File` or of the `RESOURCE` line.
- **Expected:** a checked diagnostic along the lines of
  `error[2-203-XXXX]: resource type 'File' collides with a built-in resource
  type`, pointing at the `RESOURCE File` declaration.

**The helper named tracks the collided type**, which confirms the mechanism
rather than merely the symptom:

| Declaration | Observed error |
| --- | --- |
| `RESOURCE File` | `NIR declares unused runtime helper 'fs'` ✗ |
| `RESOURCE Socket` | `NIR declares unused runtime helper 'net'` ✗ |
| `RESOURCE Db` (no collision) | builds and runs ✓ |

**Contrast cases that bound the bug:**

- A native resource with a non-colliding name (`Db`, `Stmt`, `SoundFile`) works
  correctly — this is what every in-tree fixture and binding uses, which is why
  the bug has gone unnoticed.
- The failure is **independent of `STATE`**: a stateful
  `AS RES File STATE Info` fails identically. That matters because the stateful
  path has been record-wrapped since plan-53-A, so this bug **predates
  plan-59-A** and is not a consequence of widening the record.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS 24.6.0 | aarch64, debug | fails ✗ |

## Root Cause

Two independently-reasonable behaviors compose into an internal error:

1. Declaring `RESOURCE File` does **not** collide-check against
   `BUILTIN_RESOURCES` (`src/builtins/resource.rs:116-220`). The user
   declaration is accepted and, for resolution purposes, the name `File` now
   denotes something the built-in machinery still believes it owns.
2. Because the built-in `File` type is considered reachable, the module declares
   the `fs` runtime helper. No call actually resolves to it — the user's own
   `CLOSE BY sql::close` is used instead — so nothing references the helper.

`validate_module` (`src/target/shared/validate.rs:107`) then enforces the
invariant that every declared runtime helper must be used, finds `fs` declared
and unreferenced, and reports it. The check is correct and is doing its job; it
is simply the first thing downstream of the missing collision check, so it is
where an unrelated-looking message surfaces.

The contrast cases are immune because a non-colliding name never causes a
built-in helper to be declared in the first place.

### The trigger is broader than name collision (added 2026-07-20)

Found while building plan-59-C's positive fixture (that plan's Correction C7):
the same internal error fires for a program that **does not shadow anything**. A
file that imports `fs` and declares `RES` parameters of type `File` — but never
*calls* an `fs::` function — fails identically:

```
error: NIR declares unused runtime helper 'fs'
```

So the general trigger is **any program in which a built-in resource helper is
declared but no call resolves to it**; name shadowing is one way to reach that
state, not the only one. Merely *referring* to a built-in resource type is enough
to declare its helper.

This widens the bug and slightly changes the fix: rejecting the name collision
(Phase 2) closes the shadowing route but **not** this one. A complete fix must
either declare the helper only when a call actually resolves to it, or not
declare it on a bare type reference. Phase 1's audit should therefore enumerate
both routes, and Phase 2's acceptance must cover the non-shadowing case above.

#### Correction (2026-07-21): the second route is real, but its cause is not this bug

The section above is **half wrong**, and the half that is wrong matters, because
it would have driven the fix into weakening the `validate.rs:107` invariant that
the Non-goals forbid touching.

*Wrong as written:* a `RES` **parameter** of type `File` declares nothing.
`required_helpers` walks only `function.body`
(`src/target/shared/runtime/usage.rs:129-131`); it never inspects params, global
bindings, or a `ForEach` loop variable's type. Verified — this program builds
clean:

```basic
IMPORT fs
FUNC useIt(RES f AS File) AS Integer
  RETURN 0
END FUNC
FUNC main() AS Integer
  RETURN 0
END FUNC
```

*Right in substance:* a non-shadowing route does exist, but it needs a local
**`Bind`**, not a parameter. `usage.rs:142-147` declares the helper for any
`Bind` whose declared type is a builtin resource. Adding one line to the program
above reproduces the internal error:

```basic
FUNC useIt(RES f AS File) AS Integer
  RES g AS File = f          ' <- this line, not the parameter
  RETURN 0
END FUNC
```

*Why the fix did not follow this section's advice.* The obvious repair — mirror
`usage.rs`'s rule in `validate.rs`'s `used_helpers`, exactly as the existing
resource-**union** block at `validate.rs:68-94` already does — is **wrong**, and
tracing it is what exposed a far more serious defect. Codegen really does emit a
close for `g` (`builder_control.rs:260-268`; the only non-owning escape hatches
are `UnionExtract`, `Capture{by_ref}`, and a floated collection — an initializer
that is a plain local reference is treated as *owning*). So the helper is
genuinely used, and the unused-helper error is **not** a false positive here: it
is a loud compile-time failure standing in front of a program that would
otherwise miscompile. Silencing it would have converted that into a runtime
fault — precisely the outcome the Non-goals forbid.

That underlying defect is now **bug-375**: `RES g AS File = f` closes the
*caller's* resource at the callee's scope exit, contradicting §15.6 ("the owning
scope closes it exactly once"). Confirmed at runtime, not by inference:

```
Error: 7-703-0004  Resource handle is already closed.   [exit 255]
```

Route 2 is therefore left deliberately unfixed here and is owned by bug-375.
This bug fixes the shadowing route only, which is what its Goal states.

## Goal

- A `RESOURCE` declaration whose name matches a `BUILTIN_RESOURCES` key is
  rejected at check time with a rule-coded diagnostic naming the type and
  pointing at the declaration.
- `validate.rs:107` is never reached by this input.

### Non-goals (must NOT change)

- **Do not make user resources shadow built-ins.** Permitting the shadow is a
  language-design change with a much larger surface (which `File` do `fs::`
  functions mean inside that module?). This bug asks only for a clean rejection.
- **Do not weaken or delete the `validate.rs:107` unused-helper check.** It is a
  correct invariant that caught this. The tempting wrong fix — suppressing the
  unused-helper error, or special-casing `fs`/`net` in it — would convert a loud
  failure into a silently mis-linked binary, and is forbidden.
- **Do not rename or reserve any currently-legal resource name.** Only exact
  collisions with the 8 `BUILTIN_RESOURCES` keys are affected.

## Blast Radius

The colliding namespace, by actual enumeration of `BUILTIN_RESOURCES`
(`src/builtins/resource.rs:116-220`) — 8 names:

`File`, `Socket`, `Listener`, `UdpSocket`, `TlsSocket`, `TlsListener`,
`AudioInput`, `AudioOutput`

- `src/builtins/resource.rs:258` (`is_builtin_resource`) — the predicate the fix
  calls; already exists and needs no change.
- The `RESOURCE` declaration check (wherever `RESOURCE_CLOSE_MISSING` /
  `RESOURCE_CLOSE_SIGNATURE` are emitted, `src/rules/table.rs:1004-1010`) — the
  natural home for the new check, since it already validates the declaration.
- `src/target/shared/validate.rs:107` — where the symptom surfaces today;
  unaffected by the fix beyond no longer being reached by this input.
- In-tree bindings (`bindings/libsnd` `SoundFile`, `bindings/sqlite3` `Db`/`Stmt`)
  and all 18 `tests/rt-behavior/native/` fixtures — unaffected: none uses a
  colliding name. Verified by grepping the `RESOURCE` declarations in each.

## Fix Design

Add a collision check where `RESOURCE` declarations are already validated: if
`is_builtin_resource(name)`, emit a new rule in the `2-203-xxxx` type family
(next free code; read the surrounding rows and never recycle a retired one).

The correctness risk is *placement*, not logic: the check must run early enough
that no built-in helper has been declared on the strength of the shadowed name,
otherwise the internal error still wins the race and the new diagnostic is never
seen. Phase 1's test failing for the *documented* reason is what pins this.

**Rejected alternative — special-case the unused-helper check to tolerate a
shadowed built-in.** Treats the symptom, leaves the program semantically
ambiguous, and weakens an invariant that is currently doing useful work.

**Rejected alternative — auto-namespace the user resource.** Silently renaming a
user's type is worse than refusing it.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/syntax/resources/resource-shadows-builtin-invalid` reproducing
      the `RESOURCE File` case; confirm it fails today with the *internal* error
      (`NIR declares unused runtime helper 'fs'`), which is the documented wrong
      behavior.
- [x] Add a positive contrast fixture with a non-colliding native resource name,
      guarding against the fix over-rejecting.
- [x] Confirm the 8-name blast radius above by grepping every in-tree `RESOURCE`
      declaration for collisions; record the verdict here.

Reproduction confirmed before any change, all three rows of the table above
exactly as filed: `RESOURCE File` → `unused runtime helper 'fs'`,
`RESOURCE Socket` → `'net'`, `RESOURCE Db` → builds and runs.

No new positive fixture was added: `native-resource-link-valid` already *is* the
contrast the phase asks for — the same native-resource shape (`LINK` block,
`CLOSE BY`, `RES` return) under the non-colliding names `Db`/`Stmt`. Adding a
second would duplicate it. It still compiles after the fix.

Audit verdict — every `RESOURCE` declaration in `tests/`, `bindings/`,
`examples/`, and `src/docs/`, by name and count:

| Name | Occurrences | Collides? |
| --- | --- | --- |
| `Db` | 27 | no |
| `Stmt` | 7 | no |
| `SoundFile` | 6 | no |
| `Handle` | 2 | no |
| `Tracked`, `T`, `SfFile` | 1 each | no |

**Zero in-tree collisions with any of the 8 built-in names**, confirming the
Blast Radius section. Independently re-checked by building the repro under each
of the 8 built-in names (all rejected) plus `Db`/`SoundFile`/`Stmt`/`MyFile`
(all still build) — so the check is exactly as wide as intended, with no
over-rejection.

Commit: —

### Phase 2 — the fix

- [x] Add the collision rule to `src/rules/table.rs` (next free `2-203-xxxx`).
- [x] Emit it from the `RESOURCE` declaration check when
      `is_builtin_resource(name)` holds.
- [x] Document it in `src/docs/spec/diagnostics/01_rule-codes.md` and in the
      resource-management spec's declaration section.

Landed as `2-203-0134 RESOURCE_SHADOWS_BUILTIN`. `0134` is the next free code:
`0054` and `0057` are retired gaps and were deliberately not recycled.

Two notes where the plan's own references were slightly off:

- The predicate is `is_builtin_resource_type`
  (`src/builtins/resource.rs:257`), reached through the crate-level wrapper
  `builtins::is_resource_type` (`src/builtins/mod.rs:121`) — not
  `is_builtin_resource`. No change to it was needed, as predicted.
- The emission site is **not** where `RESOURCE_CLOSE_*` are emitted (those are in
  the resolver, `src/resolver/resolution.rs:496`, which has no access to the
  built-in resource table). It is `SyntaxChecker::check_resource_decl`
  (`src/syntaxcheck/mod.rs:423`) — a hook that already existed, was already
  wired into `check()`, and was empty. It runs long before lowering, so the
  placement risk the Fix Design flagged does not arise: `validate.rs:107` is
  never reached.

Acceptance met: the negative fixture reports `2-203-0134` twice with spans on
lines 14 and 15; `native-resource-link-valid` still compiles;
`validate.rs:107` is not reached.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [x] Seed goldens for both new fixtures.
- [x] `cargo test --bin mfb spec` — `every_rule_is_documented_in_the_spec`,
      `spec_links_resolve`, `spec_citations_resolve`.
- [x] `cargo test`; `scripts/test-accept.sh target/debug/mfb <tmp> 'resource*'
      'native*'` with a hermetic `MFB_HOME`.

One golden, not two (only one fixture was added — see Phase 1). `sync-goldens.sh`
never *creates* goldens, so `golden/build.log` was seeded empty first, then
synced.

- `cargo test --bin mfb spec` — 48 passed, 0 failed.
- `cargo test` — 3138 passed, 0 failed, 1 ignored, plus all integration
  binaries green.
- `scripts/test-accept.sh … 'resource*' 'native*'` under a hermetic `MFB_HOME` —
  107 tests ran, all passed.
- `cargo fmt --check` — the two files this bug touched are clean. Three
  unrelated files (`arch/x86_64/encode/tests.rs`, `ir/verify/mod.rs`,
  `target/shared/code/link_thunk.rs`) carry pre-existing diffs from other work
  in this shared tree and were deliberately left untouched.

Acceptance met: full suite green; the only golden delta is the new fixture.
Commit: —

## Validation Plan

- Regression test: `tests/syntax/resources/resource-shadows-builtin-invalid`,
  failing with the internal error before and the new rule code after.
- Runtime proof: none needed — this is a compile-time diagnostic. The positive
  contrast fixture compiling *is* the proof the fix does not over-reject.
- Doc sync: `diagnostics/01_rule-codes.md` plus the resource-management spec.
- Full suite: `cargo test`; `scripts/test-accept.sh` for `resource*` `native*`.

## Open Decisions

- ~~**Should all 8 built-in names be rejected, or only those whose helper is
  actually reachable in the current module?**~~ **Resolved as recommended:** all
  8 are rejected unconditionally, independent of whether the helper would be
  reachable. Verified by building the repro under each of the 8 names. The
  reasoning in the recommendation held up — conditional rejection would make the
  diagnostic appear only once an unrelated import pulled the helper back in,
  which is the least predictable possible rule.

## Summary

The engineering risk is placement — the new check must fire before any built-in
helper is declared on the strength of the shadowed name, or the internal error
still wins and the diagnostic is dead code. The logic itself is one call to an
existing predicate.

Left untouched: the `validate.rs:107` unused-helper invariant (which is correct
and caught this), every non-colliding resource name, and the question of whether
shadowing should ever be *permitted* — this bug only makes the refusal legible.

**Outcome:** the placement risk did not materialize — the emission site
(`syntaxcheck::check_resource_decl`) runs long before lowering, so the new
diagnostic always wins. The real finding was elsewhere: chasing this bug's
"route 2" showed the `validate.rs:107` error is not a false positive but a
compile-time barrier in front of a genuine miscompile, now filed as **bug-375**
(`RES g AS T = f` closes the caller's resource). The Non-goal that forbade
weakening that check turned out to be load-bearing for a reason the original
report did not know.
