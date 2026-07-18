# bug-278: `mfb audit` capability/fallibility/resource tables lag the current builtin surface (bug-96 recurrence)

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (security-tooling under-reporting)

Status: Open
Regression Test: tests/ audit fixtures (new) — audio/term/resourcePath/openWithin/http.server/conversions/source-package projects surface capabilities, resources, and fallible flow

`mfb audit`'s three hand-maintained builtin tables in
`src/audit/collect/source.rs` — capability disclosure (`builtin_capability`),
fallibility (`is_fallible_call` / `is_fallible_builtin`), and resource production
(`resource_producer`) — have fallen behind every builtin surface added since
bug-96 extended them for tls/http/crypto/datetime. bug-96's own recommendation
(derive the tables from the builtin registry) was not taken, so new builtins fall
through all three tables and audit silently under-reports. This is the exact
supply-chain/auditability guarantee the MVP leans on, so silent under-reporting is
dangerous: a project that captures the microphone, controls the terminal, opens
files/sockets, or can raise runtime errors can audit as completely benign.

The single correct behavior a fix produces: every capability-bearing, fallible, or
resource-producing builtin currently in the registry is reflected in the audit
output — `audio::openInput` discloses microphone capability, `fs::openWithin`
produces a Resources row, `toInt(s)` marks its caller fallible, etc.

Confirmed gaps (live repro against `target/debug/mfb audit`):
- Capability: entire `audio` package (incl. `audio::openInput` = mic capture) and
  `term` package disclose nothing; `os::resourcePath` (reads `/proc/self/exe`,
  sibling of the mapped `os::executablePath`) discloses nothing.
- Fallibility: `audio::*` (documented Errors), `os::resourcePath`
  (ErrInvalidPath/ErrUnsupported), all bare `general` conversions
  (`toInt`/`toFloat`/… raise ErrInvalidFormat/ErrOverflow), and every
  source-package builtin (`collections::get` OOB, `regex::match` ErrInvalidFormat,
  csv/encoding/money) are treated as pure.
- Resource producers: `fs::openWithin` (returns `File`, added by bug-259),
  `http::server` (returns `net::Listener`; bug-96 added only `http::serverSSL`),
  `audio::openOutput`/`openInput` (AudioOutput/AudioInput closed by `audio::close`)
  produce no Resources rows and no AUDIT-RESOURCE-CLOSE-MAY-FAIL findings.

References:

- `bugs/completed-bugs/bug-96-audit-collector-missing-tls-http-crypto.md`
  (prior recurrence; proposed registry-derivation, not adopted).
- `src/docs/spec/tooling/08_auditability.md` ("Every fallible call site …").
- Found during goal-06 review of `src/audit/collect/source.rs`.

## Failing Reproduction

```
# Project with: RES out = audio::openOutput(48000, 2, 512)
mfb audit .
```

- Observed: audits as completely empty — 0 permissions, 0 resources, 0 findings,
  no fallible flow. Likewise `LET h = fs::openWithin("/tmp","a.txt")` +
  `LET l = http::server(8080)` produce an empty Resources section; a FUNC whose
  only error source is `toInt(s)` / `collections::get(items,99)` /
  `regex::match(s,"[bad")` / `os::resourcePath("../escape")` is reported pure; a
  term-only SUB discloses no terminal capability.
- Expected: microphone/terminal/exe-path capabilities disclosed, Resources rows +
  close-may-fail findings for the produced resources, callers marked fallible.

## Root Cause

`src/audit/collect/source.rs:539` (`builtin_capability`), `:626`
(`is_fallible_call`), `:645` (`is_fallible_builtin`), `:688`
(`resource_producer`): three hand-maintained match/lookup tables not updated as
builtins were added; no single source of truth ties them to the registry.

## Goal

- Extend all three tables with the confirmed missing entries above so the repro
  projects audit correctly.
- (Recommended, larger) Derive the tables from the builtin registry / package
  `FAIL` sites so the next builtin cannot silently fall out.

### Non-goals (must NOT change)

- The audit report format / finding codes.
- Over-reporting: do not mark genuinely-pure builtins fallible.

## Blast Radius

- The four cited tables — fixed by this bug.
- Related audit under-reporting via inline-TRAP handling and internal-file
  inclusion are separate root causes — bug-279 (internal source leak), bug-280
  (inline-trap labeling), bug-283 (LOW cluster: inline-trap resource recursion,
  libraries/resources sections).

## Fix Design

Short term: add the confirmed entries (audio/term capabilities; audio/conversions/
source-package fallibility; fs::openWithin/http::server/audio resources). Long
term: generate the tables from `src/builtins/*.rs` registration metadata (each
builtin already declares purity/resource/STATE flags — see the clean
`src/builtins/resource.rs` seed) so audit and codegen share one truth. Recommend
landing the table extensions first (unblocks correctness), then the derivation as
a follow-up.

## Phases

### Phase 1 — failing fixtures + audit
- [ ] Add audit acceptance fixtures for the repro projects; confirm they
      under-report today.
### Phase 2 — the fix
- [ ] Extend the three tables; (optional) wire registry derivation.
### Phase 3 — validation
- [ ] Regenerate audit goldens; confirm only the intended new rows appear; full
      suite green.

## Validation Plan

- Regression: the new fixtures.
- Runtime proof: the repro projects now disclose capabilities/resources/fallibility.
- Doc sync: none beyond keeping 08_auditability.md accurate (see bug-283 F6).

## Summary

The audit tables are a manually-synchronized list that has drifted twice now;
extending them restores correctness and the registry-derivation follow-up prevents
the third recurrence. Value is high for the MVP's supply-chain story.
