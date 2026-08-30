# bug-462: macOS `tls::listen` CFReleases uninitialized stack slots on every failure path

Last updated: 2026-08-30
Effort: small (< 1h)
Severity: HIGH
Class: Memory safety (release of an uninitialized pointer; process trap)

Status: Fixed (plan-110-F Phase 2)
Regression Test:
`codegen::builtins::tls::gen_macos::tests::listen_zeroes_its_cleanup_slots_before_any_failure_exit`

## Symptom

On macOS, `tls::listen` with a passphrase-protected key does not raise — it **kills the process**:

```
$ ./server.out
$ echo $?
133                          # 128 + SIGTRAP

EXC_BREAKPOINT (SIGTRAP)
  CoreFoundation  CF_IS_OBJC
  CoreFoundation  CFRelease
  <program>
```

A `TRAP` around the call cannot catch it: the process is gone before any handler runs.

## Root cause

`lower_tls_listen_macos`'s `cert_fail` exit best-effort-releases the four CoreFoundation objects a
PEM import may be holding — `CERTREF`, `KEYREF`, `ITEMS`, `DATA` — each guarded on being non-NULL
and then cleared, so that an exit taken before a slot was filled is a no-op. Its comment said as
much:

> `read_fail_fd` falls through to here, where all four slots are still NULL — a no-op.

Nothing made that true. The prologue stored only the four arguments (`HOST`, `PORT`, `CERT`,
`KEY`); the cleanup slots at `+168`, `+176`, `+184` and `+192` were never initialized and held
whatever the caller had left on the stack. The NULL guard then passed on garbage and `CFRelease`
dereferenced it.

Every failure before both refs are set takes that exit — an unreadable cert, a malformed PEM, an
encrypted key, a cert/key pair that does not parse. That is a server's misconfiguration path: the
one an operator is most likely to hit, on first deployment, and the one where a clear error matters
most.

This is bug-236's cleanup resting on an invariant bug-236 never established. It is pre-existing, not
introduced by plan-110; it went unnoticed because `tls::listen` had no runtime test at all until
plan-110-F Phase 1 added one.

## Fix

NULL the four slots in the prologue, before anything can branch to `cert_fail`:

```rust
ins.extend(
    [DATA, ITEMS, CERTREF, KEYREF]
        .map(|slot| abi::store_u64(abi::ZERO, abi::stack_pointer(), slot)),
);
```

The regression test asserts each slot is zeroed *before* the first branch that can reach a
CFRelease-ing exit, and hardcodes the offsets deliberately, so adding a slot to the release list
without zeroing it fails here.

Measured after the fix, macOS aarch64, every `tls::listen` failure exit:

```
missingCert=raised 77070008
missingKey=raised 77070008
garbageCert=raised 77070008
encryptedKey=raised 77070008      <- was: SIGTRAP, exit 133
mismatched=accepted               <- see below
ok=accepted
```

## Related finding, not a defect

`mismatched=accepted`: a cert and key that both parse but do not belong together are accepted at
listen. No backend verifies the pair while building the credential. The mismatch surfaces on the
first connection as an `ErrTlsFailed` from `tls::accept`, with the client reporting
`tls_process_cert_verify: bad signature` — clean and catchable, just later than `tls::listen`'s
description claimed. The description was corrected rather than the behaviour changed.
