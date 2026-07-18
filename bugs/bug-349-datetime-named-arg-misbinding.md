# bug-349: `datetime::instant`/`duration` bind named arguments to the wrong slot — `instant(days := 5)` silently means 5 *seconds*

Last updated: 2026-07-18
Effort: small (<1h)
Severity: HIGH
Class: Correctness (silent wrong value in a documented public API)

Status: Open
Regression Test: a new `tests/rt-behavior/datetime/` fixture asserting `datetime::instant(days := 5)` is rejected or equals 5 days

`datetime::instant` and `datetime::duration` are overloaded by argument count
with **trailing-aligned** parameter lists — the 1-argument form is
`(seconds)`, the 5-argument form is `(days, hours, mins, seconds, nanos)`, and
each shorter form drops components off the *front*. But
`src/builtins/datetime.rs:141` describes them with a single merged,
**leading-aligned** positional table
`&[&["days"], &["hours"], &["mins"], &["seconds"], &["nanos"]]`.

So `datetime::instant(days := 5)` compiles cleanly, binds `days` to position 0,
and lowers to `__datetime_instant1(5)` — five **seconds**, off by a factor of
86,400. There is no diagnostic. Nothing in the type checker, IR verifier, or
existing guard tests catches it, and the man page documents the *correct*
trailing alignment, so a user reading the docs and writing the natural spelling
gets a silently wrong timestamp.

The single correct behavior a fix produces: a named argument binds to the
parameter of that name **in the overload the call actually selects** — so
`instant(days := 5)` is either a 5-day instant or an explicit arity error, never
5 seconds.

References:

- `src/builtins/datetime.rs:139-143` — the merged table and the comment that
  states the false premise.
- `src/builtins/datetime.rs:187-192` — `call_param_name_overloads`, the correct
  mechanism, currently used only for `FIXED_OFFSET`.
- `src/builtins/datetime_package.mfb:113, 117, 121, 125, 129` — the `instant`
  overloads; `:137, 141, 145, 149, 153` — `duration`.
- `src/builtins/datetime.rs:319-329` — `implementation_name`, the arity-keyed
  dispatch (`format!("__datetime_instant{argc}")`).
- `src/builtins/mod.rs:484-511` — `call_param_name_overloads` +
  `select_param_name_overload`, the "select the overload first, then bind names
  within it" path.
- `src/docs/man/builtins/datetime/instant.md` — Synopsis documents the correct
  trailing alignment.
- `bugs/completed-bugs/bug-94-datetime-fixedoffset-named-arg-cross-overload.md`
  — the same class, fixed for `fixedOffset`; and bug-28 (`net.connectTcp`)
  before it.
- Found during the cleanup-focused source review (worktree `cleanup-review`).

## Failing Reproduction

```sh
mkdir -p /tmp/dtprobe/src
cat > /tmp/dtprobe/project.json <<'JSON'
{ "name": "dtprobe", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > /tmp/dtprobe/src/main.mfb <<'MFB'
IMPORT io
IMPORT datetime

FUNC main() AS Integer
  LET a AS Instant = datetime::instant(days := 5)
  io::print("instant(days := 5)  toNanos:")
  io::print(toString(datetime::toNanos(a)))
  LET b AS Instant = datetime::instant(5)
  io::print("instant(5)          toNanos:")
  io::print(toString(datetime::toNanos(b)))
  LET c AS Instant = datetime::instant(days := 1, hours := 2)
  io::print("instant(days:=1, hours:=2) toNanos:")
  io::print(toString(datetime::toNanos(c)))
  RETURN 0
END FUNC
MFB
target/debug/mfb build -q /tmp/dtprobe
/tmp/dtprobe/build/dtprobe.out
```

- Observed (2026-07-18, `b12213d2`, macOS aarch64) — **compiles with zero
  diagnostics**:

```
instant(days := 5)  toNanos:
5000000000
instant(5)          toNanos:
5000000000
instant(days:=1, hours:=2) toNanos:
1000000002
```

- Expected:
  - `instant(days := 5)` = 5 days = 432,000 s = `432000000000000` ns.
    Observed `5000000000` ns = 5 seconds. **Off by 86,400×.**
  - `instant(days := 1, hours := 2)` = 1 day + 2 hours = 93,600 s =
    `93600000000000` ns.
    Observed `1000000002` ns = 1 second + 2 nanoseconds. Here `days` bound the
    2-arg form's `seconds` slot and `hours` bound its `nanos` slot — two wrong
    bindings in one call, spanning two different units.
  - `instant(5)` = 5 seconds = `5000000000` ns. **This one is correct** — and
    it is byte-identical to the `days := 5` result, which is the proof: the named
    form silently degenerated into the positional one.

Contrast — the merged table is not *uniformly* permissive. A name that is not in
a leading position is caught:

```
LET c AS Duration = datetime::duration(hours := 3)
```

```
error[2-203-0022 TYPE_CALL_ARITY_MISMATCH]: function call has the wrong number of arguments
    Call to `datetime.duration` omits parameter `days` before a later supplied argument.
```

This is what makes the bug dangerous rather than merely broken: the compiler
rejects the *harmless* spellings (a mid-list name, which would have been an
arity error anyway) and accepts exactly the ones that produce a wrong value —
those that use the **leading** names `days`, `days`+`hours`, and so on. The
guard test that exists for this class passes for the same reason (see Root
Cause).

Contrast (correct today): `datetime::parse` is also arity-dispatched
(`implementation_name` at `src/builtins/datetime.rs:325`) and *also* uses a
merged table `&[&["value"], &["pattern"], &["zone"]]`
(`src/builtins/datetime.rs:154`) — but its overloads are genuinely
**leading**-aligned (`__datetime_parse2(value, pattern)` at
`datetime_package.mfb:915`, `__datetime_parse3(value, pattern, zone)` at `:911`),
so the merged table is correct there and `parse` is immune.

| Environment | Details | Result |
| --- | --- | --- |
| macOS aarch64 | darwin 24.6.0, `target/debug/mfb` at `b12213d2` | fails ✗ |
| All targets | the defect is in `src/builtins/`, above codegen | fails ✗ (platform-independent) |

## Root Cause

`src/builtins/datetime.rs:call_param_names` returns, for `INSTANT | DURATION`
(`:141`):

```rust
INSTANT | DURATION => &[&["days"], &["hours"], &["mins"], &["seconds"], &["nanos"]],
```

preceded by the comment at `:139-140`:

```rust
// Overloaded/component constructors: name parameters by their maximal
// arity. Overload selection is by count, so the leading names line up.
```

That premise is false. `src/builtins/datetime.rs:implementation_name` (`:319-329`)
selects `format!("__datetime_instant{argc}")`, and the overloads in
`src/builtins/datetime_package.mfb` are:

| arity | signature | source |
| --- | --- | --- |
| 1 | `(seconds)` | `datetime_package.mfb:113` |
| 2 | `(seconds, nanos)` | `:117` |
| 3 | `(mins, seconds, nanos)` | `:121` |
| 4 | `(hours, mins, seconds, nanos)` | `:125` |
| 5 | `(days, hours, mins, seconds, nanos)` | `:129` |

Components drop off the **front**, not the back. So the merged table's position 0
means `days` only in the 5-argument form; in the 1-argument form position 0 is
`seconds`, in the 3-argument form it is `mins`. `duration` is identical
(`:137`–`:153`). Only the maximal-arity form binds correctly — every one of the
other four arities misbinds.

**Why the existing guard does not fire.** `src/builtins/mod.rs:589` —
`no_named_argument_alias_repeats_across_positions` — asserts only that no alias
appears in *two* position groups (the bug-28/`timeoutMs` shape). In the `instant`
table each of `days`/`hours`/`mins`/`seconds`/`nanos` appears at exactly one
position, so the table is well-formed by that test's definition and it passes.
The test checks for *ambiguity*, not for *positional truth*; a merged table can
be unambiguous and still describe the wrong parameter, which is exactly what
happens here.

**The mechanism that already exists.**
`src/builtins/mod.rs:call_param_name_overloads` (`:484`) and
`select_param_name_overload` (`:497`) implement the correct discipline — pick the
overload by `positional_count + names.len()`, then bind names *within* that
overload. `src/builtins/mod.rs:overloaded_param_name_tables_are_well_formed`
(`:614`) even asserts that a builtin declaring a per-overload table must not also
declare a merged one. `fixedOffset` was migrated to it for bug-94;
`instant`/`duration` were left on the merged table because the comment at `:139`
asserted they were safe. The comment is the bug's proximate cause: it records a
conclusion that was never checked against `datetime_package.mfb`.

## Goal

- `datetime::instant(days := 5)` yields 5 days, or a diagnostic — never 5 seconds.
- Every named-argument spelling of `instant`/`duration` binds to the parameter of
  that name in the selected overload, at every arity 1–5.
- The false comment at `src/builtins/datetime.rs:139-140` is removed.
- A guard test exists that would have caught this — i.e. one that checks
  positional *truth* against `datetime_package.mfb`, not just alias uniqueness.

### Non-goals (must NOT change)

- Positional calls. `instant(5)`, `instant(1, 2, 3, 4, 5)`, and every arity in
  between must be byte-identical before and after.
- The overload signatures in `src/builtins/datetime_package.mfb`. The trailing
  alignment is the documented, man-page-published design; do **not** "fix" this by
  reordering the `.mfb` parameters to make the merged table true — that would
  silently change the meaning of existing positional code.
- `datetime::parse`, `datetime::time`, `datetime::date` — see Blast Radius.
- Do **not** close this by editing the man page to describe the buggy binding.

## Blast Radius

The task's real question — *which other builtins have multiple overloads and a
merged `call_param_names` entry?* — answered by search, not memory.

**Step 1 — enumerate arity-dispatched overload families.**
`grep -rn '{argc}' src/builtins/*.rs` returns exactly four lines, all in
`datetime.rs`:

- `src/builtins/datetime.rs:322` — `INSTANT` → `__datetime_instant{argc}`
- `src/builtins/datetime.rs:323` — `DURATION` → `__datetime_duration{argc}`
- `src/builtins/datetime.rs:324` — `FIXED_OFFSET` → `__datetime_fixedOffset{argc}`
- `src/builtins/datetime.rs:325` — `PARSE` → `__datetime_parse{argc}`

`grep -rn 'argc' src/builtins/*.rs` outside `datetime.rs` returns **nothing**, so
no other module dispatches on argument count by any other spelling.

**Step 2 — enumerate builtins already on a per-overload table.**
`grep -rln 'fn call_param_name_overloads' src/builtins/` → `mod.rs` (the
aggregator), `net.rs`, `audio.rs`, `datetime.rs`.

**Verdict per site:**

- `datetime::instant` (`datetime.rs:141`) — **fixed by this bug.** Trailing-aligned,
  merged table, misbinds at arities 1–4.
- `datetime::duration` (`datetime.rs:141`) — **fixed by this bug.** Identical
  shape; shares the table entry.
- `datetime::fixedOffset` (`datetime.rs:189`) — **already correct.** Returns
  `None` from `call_param_names` and declares
  `&[&["offsetSeconds"], &["hours", "mins"]]` via `call_param_name_overloads`.
  Fixed by bug-94; this bug is the sibling that migration missed.
- `datetime::parse` (`datetime.rs:154`) — **unaffected.** Also arity-dispatched
  and also on a merged table, but its overloads are leading-aligned
  (`__datetime_parse2(value, pattern)` ⊂ `__datetime_parse3(value, pattern,
  zone)`, `datetime_package.mfb:911, 915`), so `&[&["value"], &["pattern"],
  &["zone"]]` is positionally true at both arities. Add a guard, not a fix.
- `net::connectTcp` (`net.rs:138-143`) — **already correct.** Four-entry
  per-overload table; fixed by bug-28.
- `audio::openInput` / `audio::openOutput` (`audio.rs:227-230`) — **already
  correct.** Two-entry per-overload table; note this pair *is* trailing-aligned
  (`device` prepended in the 4-arg form) and would exhibit exactly this bug on a
  merged table. It is immune only because someone already did this migration.
- `datetime::time` (`datetime.rs:143`) — **unaffected**, and worth stating why:
  it varies arity via `default_argument_padding` (`datetime.rs:335-344`), which
  appends **trailing** defaults (`second`, `nanos` → 0). Trailing padding
  preserves leading alignment, so its merged table `&[&["hour"], &["minute"],
  &["second"], &["nanos"]]` is positionally true at every arity. This is the
  general rule: *`default_argument_padding` is safe; front-dropping overload
  families are not.*
- Every other builtin with a `call_param_names` entry (the 22 modules aggregated
  at `src/builtins/mod.rs:513-535`) — **unaffected**: none dispatches on arity, so
  each has exactly one positional layout and a merged table is trivially true.

So the audit closes cleanly: `instant` and `duration` are the only two remaining
instances of this class in the tree.

## Fix Design

Migrate `INSTANT | DURATION` to the per-overload mechanism, exactly as
`fixedOffset` was.

In `src/builtins/datetime.rs:call_param_names`, replace the `INSTANT | DURATION`
arm with `INSTANT | DURATION => return None`, carrying a comment that states the
*checked* fact (components drop off the front; see `datetime_package.mfb:113-129`)
rather than the assumption it replaces. In `call_param_name_overloads` (`:187`),
add:

```rust
INSTANT | DURATION => Some(&[
    &["seconds"],
    &["seconds", "nanos"],
    &["mins", "seconds", "nanos"],
    &["hours", "mins", "seconds", "nanos"],
    &["days", "hours", "mins", "seconds", "nanos"],
]),
```

`select_param_name_overload` (`src/builtins/mod.rs:497`) then picks by
`positional_count + names.len()` and requires every supplied name to sit at an
index `>= positional_count`. Under that rule `instant(days := 5)` finds no
overload of arity 1 containing `days` and becomes a clean diagnostic; `instant(days
:= 1, hours := 2, mins := 3, seconds := 4, nanos := 5)` selects the 5-arg form and
binds correctly. The existing well-formedness test at `src/builtins/mod.rs:614`
already asserts a builtin cannot carry both tables, so it will enforce the
`return None`.

Rejected: reordering the `.mfb` overload parameters to be leading-aligned. It
would make the merged table true but silently reinterpret every existing
positional call — a far worse defect than the one being fixed, and it contradicts
the published man-page Synopsis.

Rejected: leaving the merged table and adding a special-case rejection for
non-maximal arities with named args. It fixes the symptom for these two functions
and leaves the general hazard live for the next front-dropping overload family.

**Expected shift:** `instant(days := 5)` moves from silently-wrong-value to a
compile error. That is a deliberate behavior change and is the point. Positional
calls are untouched. No goldens should move — search the fixtures for named-arg
`instant`/`duration` calls in Phase 1 to confirm.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a fixture reproducing `instant(days := 5)` and `instant(days := 1,
      hours := 2)`; confirm the wrong values `5000000000` and `1000000002`.
- [ ] `grep -rn 'instant(.*:=\|duration(.*:=' tests/ examples/ src/` to find any
      existing source relying on the buggy binding; each hit must be inspected,
      since Phase 2 turns it into a compile error.
- [ ] Complete the blast-radius audit (done above); write the verdicts into this
      file.

Acceptance: the fixture fails with the documented wrong values; the audit is
complete with a verdict per site; the list of affected existing call sites is
known.
Commit: —

### Phase 2 — the fix

- [ ] `src/builtins/datetime.rs:141` → `INSTANT | DURATION => return None`, and
      delete the false comment at `:139-140`.
- [ ] Add the five-overload table to `call_param_name_overloads`
      (`src/builtins/datetime.rs:187`).
- [ ] Fix any call site found in Phase 1.

Acceptance: the Phase 1 fixture passes; positional `instant`/`duration` calls at
every arity 1–5 are unchanged; `cargo test` green including
`overloaded_param_name_tables_are_well_formed`.
Commit: —

### Phase 3 — close the class + full validation

- [ ] Add a guard test that checks each arity-dispatched builtin's declared
      parameter names against the actual `__pkg_nameN` signatures in the package
      source. The existing test at `src/builtins/mod.rs:589` checks alias
      uniqueness only and passes on this bug; the new one must fail on it. This is
      what stops a fourth instance of bug-28/94/349.
- [ ] Re-run the reproduction end-to-end and confirm the diagnostic.
- [ ] Full acceptance:
      `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- [ ] Confirm no golden moved.

Acceptance: the new guard fails against the pre-fix table and passes after; full
suite green; zero golden churn.
Commit: —

## Validation Plan

- Regression test(s): a `tests/rt-behavior/datetime/` fixture for the value, plus
  a `tests/syntax/datetime/` fixture asserting `instant(days := 5)` is now
  diagnosed; and the Phase 3 unit guard in `src/builtins/mod.rs`.
- Runtime proof: the reproduction above — `instant(days := 5)` must stop printing
  `5000000000`.
- Doc sync: none expected. `src/docs/man/builtins/datetime/instant.md` already
  documents the correct trailing alignment; it is the code that drifted from the
  man page. Re-read it after the fix to confirm.
- Full suite: `cargo test` and
  `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Should `instant(days := 5)` become an error, or bind to the 5-arg overload
  with the other four components defaulting to 0?** Recommend the **error**, for
  consistency with `fixedOffset`/`connectTcp` and because
  `select_param_name_overload` already produces it with no new machinery.
  Silently defaulting four components would re-introduce a "looks like it worked"
  path, which is the failure mode this bug is about. Revisit only if it proves
  ergonomically painful in real code.

## Severity reasoning

HIGH. This is a silent wrong value in a documented public API: it compiles with
zero diagnostics, the wrong result is 86,400× off, and the *only* named-argument
spellings the compiler accepts are the ones that misbind. It affects four of five
arities on two functions, is platform-independent, and the guard test written
specifically to catch this class passes over it. bug-94 rated the same class MED,
but that instance had a single wrong overload pair and a 3,600× error; this one
spans eight broken (function, arity) combinations and reaches the most natural
spelling of the highest-arity form. Time values that are wrong by a day, with no
error, will surface as data corruption far from the call site.

## Summary

The engineering risk is not the fix — it is a mechanical migration to a mechanism
that already exists and is already exercised by three other builtins. The risk is
in Phase 1 (finding existing call sites that Phase 2 turns into compile errors)
and Phase 3 (writing the guard that would have caught this, since the existing one
checks alias uniqueness rather than positional truth). The audit's conclusion is
the reassuring part: `instant` and `duration` are the last two instances of this
class in the tree, and the general rule separating safe from unsafe cases is
whether arity varies by trailing `default_argument_padding` (safe) or by a
front-dropping overload family (not).
