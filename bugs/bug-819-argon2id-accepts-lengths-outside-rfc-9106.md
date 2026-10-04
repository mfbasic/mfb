# bug-819: `crypto::argon2id` accepts parameters outside RFC 9106

Last updated: 2026-10-04
Effort: small–medium (guess)
Severity: LOW
Class: Correctness (standards conformance)

Status: Open
Regression Test: proposed — a runtime test for each out-of-range parameter.

Found while fixing bug-814. Owner decision (2026-10-04): follow the RFC.

## Problem

`crypto::argon2id` accepts an output `length` above 2^32−1, which RFC 9106
§3.1 forbids (tag length T must be 4..2^32−1 bytes). The man page says only
"at least 4".

## Goal

Every argon2id input the API exposes is checked against RFC 9106 §3.1, and an
out-of-range value raises `ErrInvalidArgument` with a message naming the
parameter:
- tag length T: 4..2^32−1;
- parallelism p: 1..2^24−1;
- memory m (KiB): 8·p..2^32−1;
- passes t: 1..2^32−1;
- message (password) and nonce (salt) lengths: at most 2^32−1, and the nonce at
  least 8 bytes;
- secret and associated data, if exposed: at most 2^32−1.

Each bound is cited in the man page and the spec. The declared errors stay exact,
and the bug-698/807/814 guards must stay green. The output for valid parameters
does not change (RFC 9106 test vectors).

## Phases

- [ ] RED tests, one per bound; the RFC test vectors as a control.
- [ ] Add the checks; prose and spec; guards.
