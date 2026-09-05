# bug-517: `CRYPTO_SHA1_INSECURE` fires on the enum member, so it cannot tell a broken use of SHA-1 from a sound one

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Footgun

Status: Open
Regression Test: `src/ir/tests.rs` — the existing `CRYPTO_SHA1_INSECURE` filter test, extended

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
value-scoped advisory. The defect is that SHA-1's danger is call-scoped.

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
- [ ] `grep -rn "advisory:" src/codegen/builtins/` — enumerate every
      `EnumVariant::advisory` row and write the list into Blast Radius. This
      decides whether the mechanism may change shape.

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

- Whether `crypto::pbkdf2` belongs on the allow-list. PBKDF2-HMAC-SHA1 is sound
  *as a KDF*, and it is the WPA2/RFC 8018 profile — but it is also what someone
  reaches for when storing passwords, where the real advice is bug-515's
  Argon2id. **Recommend allow-listing it** (SHA-1 is not the problem there;
  PBKDF2 is), and letting bug-515 own the password-storage advice.

## Summary

The risk is in the allow-list, not the mechanism: every member added to it is a
claim that SHA-1 is sound in that construction, and a wrong entry silences a
real warning. The list is short, each entry is defensible from a published
proof, and `crypto::hash` — the case the rule exists for — is untouched.
