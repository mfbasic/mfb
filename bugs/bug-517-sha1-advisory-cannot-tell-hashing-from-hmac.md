# bug-517: `CRYPTO_SHA1_INSECURE` fires on the enum member, so it cannot tell a broken use of SHA-1 from a sound one

Last updated: 2026-09-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Footgun

Status: **FIXED** (2026-09-05). The owner ruled on the question this document
was blocked on: the advisory is **use-scoped** — `hash` warns, `hmac`/`hkdf`/
`pbkdf2` do not. That reverses `plan-109-A`'s recorded decision; see "The owner's
ruling" below for what the reversal cost and what it deliberately did not change.
Regression Test: `src/ir/tests.rs` —
`the_sha1_advisory_fires_on_hash_and_not_on_hmac_hkdf_or_pbkdf2` and
`the_sha1_advisory_suppression_is_fail_closed`; plus the extended behavioural
fixture `tests/rt-behavior/crypto/crypto-sha1-advisory-valid`.

`crypto::Hash.SHA1` carries a compile-time advisory that fires wherever the enum
member is *written*, not where it is *used*. Every one of these gets the same
warning with the same reason:

| call | is SHA-1 actually a problem here? |
| --- | --- |
| `crypto::hash(crypto::Hash.SHA1, msg)` | **yes** — the security claim is collision resistance, and it is broken |
| `crypto::hmac(crypto::Hash.SHA1, key, msg)` | no — HMAC's proof does not rest on collision resistance |
| `crypto::hkdf(crypto::Hash.SHA1, …)` | no — built on HMAC-SHA1 |
| `crypto::pbkdf2(crypto::Hash.SHA1, …)` | no — this is the RFC 8018 / WPA2 profile the page itself names as legitimate |

The warning text is "SHA-1 is not collision-resistant; use it only for legacy
interoperability", with a detail line recommending `SHA2_256`. For rows 2–4
that reason is simply not the reason, and the recommendation is wrong whenever
the peer specifies HMAC-SHA1 — RFC 6238 TOTP, WPA2, and TLS-era interop all do.

The single correct behavior a fix produces: the advisory distinguishes the use.
A bare `crypto::hash` with SHA-1 keeps the current warning; the HMAC-family
members either fall silent or report a different, accurate advisory that does
not tell the author to change an algorithm the protocol pins.

An advisory that fires on the sound uses is not merely noisy: it trains authors
to `SHA1`-warnings-are-normal, which is exactly the state in which the one
warning that matters gets skipped.

References:

- `src/ir/verify/values.rs:check_enum_member_advisory` — the emission site
- `src/codegen/builtins/crypto/mod.rs:197` — the `EnumVariant::advisory` row
- `src/rules/table.rs:759` — rule `2-203-0136`
- `src/codegen/builtins/crypto/func_hmac.rs:27-30` — the page already
  half-concedes this ("HMAC-SHA1 … still reports the advisory")
- Spike: `spikes/api-review/bug-517-sha1-advisory-context/`

## Failing Reproduction

```
./target/release/mfb build spikes/api-review/bug-517-sha1-advisory-context
```

- Observed (macOS aarch64): three warnings, textually identical, one per call —

```
…/main.mfb:21 warn[2-203-0136 CRYPTO_SHA1_INSECURE]: SHA-1 is not collision-resistant; use it only for legacy interoperability
             `crypto::Hash.SHA1` selects SHA-1, which is not collision-resistant … use `crypto::Hash.SHA2_256` or stronger for new designs.
…/main.mfb:24 warn[2-203-0136 CRYPTO_SHA1_INSECURE]: (identical)
…/main.mfb:27 warn[2-203-0136 CRYPTO_SHA1_INSECURE]: (identical)
```

  Line 21 is `crypto::hash`; lines 24 and 27 are `crypto::hmac` and
  `crypto::hkdf`.

- Expected: line 21 warns as it does today. Lines 24 and 27 do not carry a
  collision-resistance warning, because HMAC-SHA1 and HKDF-SHA1 are not broken
  by SHA-1 collisions.

Contrast case that works correctly today: the advisory is correctly suppressed
inside injected builtin source and on the package path
(`check_enum_member_advisory` returns early for `builtins/` files), so a
package's own dispatch helper comparing against every `Hash` variant does not
warn. That suppression is the model for the fix — it already proves the emitter
can be context-sensitive.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ |
| Linux / Windows | — | front-end diagnostic, target-independent; expected identical |

## The owner's ruling (2026-09-05) — use-scoped, and what that cost

Asked directly, the owner answered: **"`hash` but not `hmac`/`hkdf`/`pbkdf2`."**
So the Verdict below is superseded on its one open question — the analysis that
produced it stands, and is kept because it is the record of what the reversal
overturned.

### Implemented as SUPPRESSION at the sound consumers, not "fire only at hash"

The ruling names the members that should not warn, and there are two ways to
honour it. Firing only where the consumer is provably `crypto::hash` makes
silence the default; suppressing only where the consumer is provably one of the
three keeps *warning* the default. The second is implemented, because the two
failure directions are not symmetric: a false positive is a warning on a sound
use, a false negative is silence on a broken one.

Concretely, all of these still warn — the checker cannot see a sound consumer
from the occurrence, so it does not assume one:

| shape | why it still warns |
| --- | --- |
| `crypto::hash(Hash.SHA1, m)` | the broken use; the point of the advisory |
| `LET h = Hash.SHA1` then `hmac(h, …)` | the occurrence does not name its consumer; this is not a dataflow analysis |
| `CASE crypto::Hash.SHA1` | a `MATCH` literal is not a selector argument |
| `hmac(same(Hash.SHA1), …)` | nested INSIDE the argument, so not hmac's selector |

Pinned by `the_sha1_advisory_suppression_is_fail_closed`.

### The obvious implementation suppressed nothing

Matching the call target `"crypto.hmac"` looked correct and changed no behaviour
at all — the spike still emitted all three warnings. `crypto::hash` reaches IR
verification as the dotted `crypto.hash`, but `hmac`/`hkdf`/`pbkdf2` are
`.mfb`-bodied and arrive as **`#crypto_hmac`** — `internal_name::internalize` of
the package's own `__crypto_hmac`. `hash_selector_use_is_sound` now accepts both
forms through the mangling contract rather than a hardcoded `#` literal. Only
running the spike caught this.

### What the reversal cost, measured

Exactly what this document predicted, plus one page it did not:

- two spec paragraphs (`diagnostics/01_rule-codes.md`, `stdlib/10_crypto.md`);
- the `SHA1` variant `description`;
- **four** member pages, not three — `func_hash.rs` also needed rewriting, to say
  the advisory is scoped to the use and that it fires *here above all*;
- behavioural goldens: `crypto-sha1-advisory-valid` (extended) and
  `crypto-kat-valid` (**6 advisory blocks removed, 0 added, no program output
  changed**). `crypto-kdf-invalid` did not move at all, contrary to the estimate.

### `plan-109-A` is not edited

It is a completed plan and stays a true record of what was decided then. The
reversal is recorded here and in the code
(`ir::verify::values::hash_selector_use_is_sound`), which cites it.

## Verdict (2026-09-05) — reproduced, root-caused, and BLOCKED on a product decision

**Reproduced verbatim**, at `31df7f872`, release, macOS aarch64:

```
$ ./target/release/mfb build spikes/api-review/bug-517-sha1-advisory-context
```

emits exactly three `warn[2-203-0136 CRYPTO_SHA1_INSECURE]` — one each on lines
21 (`crypto::hash`), 24 (`crypto::hmac`) and 27 (`crypto::hkdf`) — with
byte-identical message and detail lines, and exits 0 with an executable written.
(The spike's own header comment says "read the four warnings"; the spike has
three SHA-1 call sites, not four. `crypto::pbkdf2` is named in the table above
but is not exercised by the spike.)

So the *description* of the behaviour is accurate. What is not accurate is the
Root Cause section's framing of it as an emitter limitation.

### The behaviour is a recorded decision, not an oversight

`check_enum_member_advisory` does not fail to see the call context by accident.
Four independent, mutually consistent sources say the advisory is scoped to the
**value**, deliberately:

1. **The plan that built it.** `planning/completed/plan-109-A-hash-api-sha1-warning.md`
   §3 "Design Overview", last sentence of the SHA-1 implementation paragraph:

   > Extend the generic digest/block/output helpers so HMAC/HKDF/PBKDF2 accept
   > SHA-1 consistently (20-byte output, 64-byte block); **the warning applies
   > regardless of which public function consumes the selector.**

   That sentence answers precisely the question this bug re-opens, in the
   opposite direction. `grep -n "regardless of which public function" planning/completed/plan-109-A-hash-api-sha1-warning.md`

2. **The spec, twice.**
   - `src/docs/spec/diagnostics/01_rule-codes.md:283-287` — "`CRYPTO_SHA1_INSECURE`
     is the registry's enum-value advisory: a builtin enum variant may carry an
     `EnumVariant::advisory`, and **every user-source occurrence of that value** —
     an expression or a `MATCH` literal — reports it once, while the program still
     compiles and runs."
   - `src/docs/spec/stdlib/10_crypto.md:68-73` — "**Every user-source occurrence
     of `Hash.SHA1`** (an expression or a `MATCH` literal) reports the non-fatal
     `CRYPTO_SHA1_INSECURE` warning."

   The spec is not silent here, and it does not contradict the code: it
   specifies the code.

3. **The registry prose**, `src/codegen/builtins/crypto/mod.rs:188-190` and the
   `SHA1` variant `description` — "every source use reports the
   `CRYPTO_SHA1_INSECURE` warning".

4. **The man pages the bug calls "apologies".** They are not apologies; they are
   the two-part statement a correct page has to make. `func_hmac.rs:27-30` says
   *both* that HMAC-SHA1 is sound *and* what to prefer for a new design:
   "HMAC-SHA1 remains cryptographically sound — HMAC does not rely on collision
   resistance — but `crypto::Hash.SHA1` still reports the `CRYPTO_SHA1_INSECURE`
   advisory; **prefer `SHA2_256` unless a peer requires SHA-1**." `func_hkdf.rs:21`
   and `func_pbkdf2.rs:22-24` say the same thing for their profiles. Every one of
   those sentences is true as written.

### The advisory's text is not false at an HMAC call site

This is the load-bearing point, and the bug's table overstates it. The advisory
makes two claims:

- *"SHA-1 is not collision-resistant"* (rule message, `src/rules/table.rs:757-762`)
  — unconditionally true, independent of the construction it is used in.
- *"Keep it only for legacy interoperability; use `crypto::Hash.SHA2_256` or
  stronger for new designs"* (`EnumAdvisory::detail`) — for a *new* design,
  HMAC-SHA2-256 is preferable to HMAC-SHA1, HKDF-SHA2-256 to HKDF-SHA1, and
  PBKDF2-HMAC-SHA2-256 to PBKDF2-HMAC-SHA1. The advice is right at all four
  sites.

Nowhere does the advisory claim that HMAC-SHA1 is broken. The bug's table
column, "is SHA-1 actually a problem here?", conflates *"not catastrophically
broken"* with *"not worth flagging"*. The first is a cryptographic fact — and
the package's own pages already state it. The second is a policy judgement about
how loud a deprecated-algorithm signal should be, and it is the whole of the
question here.

### What the change would actually cost

Design A (call-site suppression) is not a bounded one-function change. Landing it
means editing, in the same commit:

- two spec paragraphs that currently say "every user-source occurrence"
  (`01_rule-codes.md`, `10_crypto.md`);
- the `SHA1` variant `description` in `crypto/mod.rs`;
- three man-page descriptions (`func_hmac.rs`, `func_hkdf.rs`, `func_pbkdf2.rs`);
- and re-baselining committed **behavioural** goldens that pin the current
  counts:
  `tests/rt-behavior/crypto/crypto-kdf-invalid/golden/build.log` (6 warnings on
  HKDF/PBKDF2 lines 55-57 and 60-62), `crypto-kat-valid/golden/build.log`
  (8 warnings), and `crypto-sha1-advisory-valid/golden/build.log`.
  `grep -rln "CRYPTO_SHA1_INSECURE" tests/`

### Applying AGENTS.md's four-question gate to those goldens

1. **When/why written** — plan-109-A Phase 1/2, 2026-08-29;
   `crypto-sha1-advisory-valid` was created specifically to pin "exactly one
   named warning per user-authored occurrence, non-fatal".
2. **Behaviour protected** — every user-source occurrence of `crypto::Hash.SHA1`
   reports `CRYPTO_SHA1_INSECURE` once, and the program still builds and runs.
3. **Who depends** — the two spec paragraphs above, the four man pages that cite
   the rule code, `src/ir/tests.rs`'s advisory tests, and
   `registry::enum_variant_advisory`'s unit tests.
4. **Proof it is wrong** — **not met.** The bug supplies a correct cryptographic
   observation (HMAC's security proof does not rest on collision resistance) that
   the repo already documents, but no evidence that any sentence the compiler
   prints is untrue at the site it prints it.

Three of four. Under AGENTS.md "Not all 4 → test wins, STOP", this fix does not
proceed without an owner decision.

### What is needed to unblock

A one-line ruling on the policy question, which is genuinely open and genuinely
two-sided:

> Is `CRYPTO_SHA1_INSECURE` an **algorithm-hygiene** signal ("you selected
> SHA-1 — confirm the peer requires it"), which is what it is today and what
> plan-109-A decided; or a **defect** signal ("this construction is broken"),
> which is what this bug argues it should be?

- **Keep as-is (hygiene).** Costs the noise this bug describes. Defended by
  plan-109-A, both spec paragraphs, and the fact that every sentence emitted is
  true. Close this bug as working-as-designed and delete the "advisory"
  sentences from the three KDF pages only if they are judged redundant.
- **Adopt Design A (defect signal).** Then the doc sync is mandatory and is
  larger than the code change: two spec paragraphs, the variant `description`,
  three man pages, and three behavioural `build.log` goldens re-baselined with
  this bug cited as the proof.

Recording the analysis rather than guessing, per the repo's rule that a
behavioural golden wins until proven wrong.

## Root Cause

`src/ir/verify/values.rs:check_enum_member_advisory` is reached from the
**member-access** arm of the value checker. Its input is
`(enum_name, member)` — `("crypto.Hash", "SHA1")` — and nothing else. It has no
view of the enclosing expression, so it cannot know whether the value it is
warning about is about to become the first argument of `crypto::hash` or of
`crypto::hmac`.

The advisory itself is a static field on the registry enum variant
(`src/codegen/builtins/crypto/mod.rs:197`, `EnumVariant::advisory`), so it is a
property of the *value*, not of any call. There is exactly one string and one
rule code for all four uses because the model has exactly one place to hang
them.

The comment at `values.rs:631` states the design intent — "report it once per
user-authored occurrence" — which is a faithful implementation of a
value-scoped advisory.

**Correction (2026-09-05).** The original last sentence of this section read
"The defect is that SHA-1's danger is call-scoped." That is the bug's
conclusion, not its root cause, and it is asserted rather than shown. The
mechanism is value-scoped *by decision* (plan-109-A §3, quoted in Verdict
above), not because the checker was unable to be otherwise — the same function
already proves it can be context-sensitive via its `builtins/` exemption. There
is no implementation defect here to root-cause; there is a policy question. See
Verdict.

## Goal

- `crypto::hash(crypto::Hash.SHA1, …)` still reports `CRYPTO_SHA1_INSECURE`
  with today's text.
- `crypto::hmac`, `crypto::hkdf` and `crypto::pbkdf2` with `crypto::Hash.SHA1`
  do not report a collision-resistance warning.
- Writing `crypto::Hash.SHA1` somewhere with no call context (assigning it to a
  variable, putting it in a `MATCH`) still reports something — silence there
  would be a regression in coverage.

### Non-goals (must NOT change)

- The severity. `CRYPTO_SHA1_INSECURE` stays `Severity::Warn` and stays
  non-fatal; the build must keep working. The user's read is that this is the
  right severity.
- The rule code `2-203-0136`, which is referenced from at least four man pages
  (`func_hash.rs`, `func_hmac.rs`, `func_hkdf.rs`, `func_pbkdf2.rs`).
- The `EnumVariant::advisory` mechanism itself, which serves other variants.
- The "once per user-authored occurrence" property — the fix must not start
  double-reporting.
- **Tempting wrong fix, forbidden:** deleting the advisory, or dropping it to
  a note, to stop the false positives. The `crypto::hash` case is the one this
  rule exists for and it must keep firing at full strength.

## Blast Radius

Every consumer of `check_enum_member_advisory`, found by
`grep -rn "CRYPTO_SHA1_INSECURE\|enum_variant_advisory" src/`:

- `src/ir/verify/values.rs:check_enum_member_advisory` — fixed by this bug.
- `src/codegen/registry/mod.rs:enum_variant_advisory` — the lookup; may need a
  call-context parameter, or a second entry point.
- `src/codegen/builtins/crypto/mod.rs:197` — the `Hash.SHA1` advisory row.
- `src/ir/tests.rs:2527` — the existing test filters on the rule name; it will
  need extending, not rewriting.
- **Any other `EnumVariant::advisory` row.** `grep -rn "advisory:"
  src/codegen/builtins/` in Phase 1 — if `Hash.SHA1` is currently the only one,
  the fix is free to change the mechanism's shape; if there are others, it must
  stay backward-compatible for them. This determines the fix design and must be
  answered first.
- `src/codegen/builtins/crypto/func_hmac.rs:27-30`, `func_hkdf.rs:21`,
  `func_pbkdf2.rs:22` — their prose currently explains away the false positive
  ("still reports the advisory; prefer `SHA2_256` unless a peer requires
  SHA-1"). Once the advisory stops firing there, those apologies are stale and
  must be rewritten.

## Fix Design

The advisory needs a call context. Two ways to get one:

**A — suppress at the call site.** Keep the value-scoped advisory, and add a
suppression when the member access is *immediately* the selector argument of a
member on an allow-list (`crypto.hmac`, `crypto.hkdf`, `crypto.pbkdf2`).
Smallest change; keeps one rule and one code. Weakness: it is syntactic, so
`LET h = crypto::Hash.SHA1` then `crypto::hmac(h, …)` still warns — but that
case *should* warn under this design, because the checker genuinely cannot see
where `h` goes. Acceptable, and it satisfies the goal's third bullet for free.

**B — move the advisory onto the function.** Give `RegistryFunction` a
per-parameter advisory keyed on the argument value, so `crypto::hash` declares
"a `SHA1` here is `CRYPTO_SHA1_INSECURE`" and `crypto::hmac` declares nothing
(or a milder note). More faithful to where the danger actually lives, and it
generalizes to the next algorithm that is fine in one construction and broken
in another. Larger: a new descriptor field and a new emission path.

**Recommend A**, with B recorded as the shape to grow into. A is a bounded
change to one function with an explicit allow-list that reads as
documentation, and it can be reversed if the allow-list turns out to need
maintenance.

Rejected: adding a second rule code (`CRYPTO_SHA1_HMAC_OK` or similar) to carry
a milder message on the HMAC members. Two codes for one algorithm invites
authors to suppress both; and per the project's rule-code hazard, claiming a
new code races with other sessions. Silence on the sound uses is the clearer
signal.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Land `spikes/api-review/bug-517-sha1-advisory-context/` (done).
- [ ] Add a diagnostic test asserting the *desired* counts: one
      `CRYPTO_SHA1_INSECURE` for `crypto::hash`, zero for `hmac`/`hkdf`/`pbkdf2`.
      Confirm it fails at 3-or-4 today. Per the diagnostic-harness rule, the
      test must record the exit status and any unlocated errors, so a failure
      cannot read as "same".
- [x] `grep -rn "advisory:" src/codegen/builtins/` — enumerate every
      `EnumVariant::advisory` row and write the list into Blast Radius. This
      decides whether the mechanism may change shape.
      **ANSWERED 2026-09-05:** `grep -rn "advisory: Some" src/codegen/builtins/
      src/codegen/registry/mod.rs` returns exactly two hits —
      `crypto/mod.rs:198` (`Hash.SHA1`, the production row) and
      `registry/mod.rs:5136` (a synthetic row inside
      `enum_variant_advisory_is_keyed_by_package_enum_and_member`). `Hash.SHA1`
      is the **only** production advisory in the tree, so the mechanism is free
      to change shape; no other variant constrains it. Every other of the 57
      `advisory:` sites is `advisory: None`.

Acceptance: the new test fails with the observed 3 warnings; the advisory-row
census is complete.
Commit: —

### Phase 2 — the fix

- [ ] Add the call-site suppression in `check_enum_member_advisory`, with the
      allow-list named and commented with *why* each member is sound.
- [ ] Rewrite the now-stale apologies in `func_hmac.rs`, `func_hkdf.rs` and
      `func_pbkdf2.rs` — they should say SHA-1 is the correct choice for the
      named legacy profiles, not that the warning is expected.

Acceptance: the Phase 1 test passes; `crypto::hash` still warns; the bare
`LET h = crypto::Hash.SHA1` case still warns.
Commit: —

### Phase 3 — full validation

- [ ] `cargo test --no-fail-fast` — diagnostics are global state, so assert by
      rule name, not by index.
- [ ] `cargo check --all-targets` at the end, for test-target warnings.
- [ ] `scripts/test-accept.sh` — any acceptance golden containing the warning
      text will shift; confirm the delta is only the removed false positives.
- [ ] `scripts/man-run-examples.sh crypto --run`.

Acceptance: full suite green; every golden delta is a removed
`CRYPTO_SHA1_INSECURE` on an HMAC-family call and nothing else.
Commit: —

## Validation Plan

- Regression test: the Phase 1 diagnostic test, asserting counts per member.
- Runtime proof: `spikes/api-review/bug-517-sha1-advisory-context/` rebuilt —
  one warning instead of three.
- Doc sync: `func_hmac.rs`, `func_hkdf.rs`, `func_pbkdf2.rs` prose;
  `func_hash.rs` keeps its paragraph; check `src/docs/spec/diagnostics` for the
  rule's description.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

**The primary decision is the one stated in Verdict above** — hygiene signal or
defect signal — and everything below is subordinate to it. Nothing here is
actionable until that is answered.

- Whether `crypto::pbkdf2` belongs on the allow-list. PBKDF2-HMAC-SHA1 is sound
  *as a KDF*, and it is the WPA2/RFC 8018 profile — but it is also what someone
  reaches for when storing passwords, where the real advice is bug-515's
  Argon2id. **Recommend allow-listing it** (SHA-1 is not the problem there;
  PBKDF2 is), and letting bug-515 own the password-storage advice.

## Summary

**As of 2026-09-05 this bug is blocked on the Verdict section's single
question, not on any of the below.** The paragraph that follows describes the
risk profile *if* Design A is chosen.

The risk is in the allow-list, not the mechanism: every member added to it is a
claim that SHA-1 is sound in that construction, and a wrong entry silences a
real warning. The list is short, each entry is defensible from a published
proof, and `crypto::hash` — the case the rule exists for — is untouched.
