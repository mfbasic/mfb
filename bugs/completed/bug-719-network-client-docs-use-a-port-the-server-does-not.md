# bug-719: network-client's TLS example and network-server's README both document `--port 7413`, which neither side's server ever listens on by default

Last updated: 2026-09-26
Effort: small (<1h)
Severity: LOW
Class: docs

STATUS: FIXED (031c2cd7d)

Fixed exactly as the doc's Fix Design recommended: dropped `--port 7413`
entirely from both the client's header comment and the server README's
paired command (7000 is the default, so no flag reads more naturally than a
redundant `--port 7000`); moved `usage()`'s `"Defaults to HOST."` line to
follow `--server-name NAME` instead of `--allow-self-signed`. No deviation.
Regression Test: none (docs-only; no test asserts printed help/comment text —
matches the doc's own Validation Plan)

`examples/network-client/src/main.mfb`'s header comment documents a TLS example
command, `network-client --port 7413 --server-name localhost --allow-self-signed`
(`examples/network-client/src/main.mfb:31`), and `examples/network-server/README.md:93`
repeats the identical client invocation verbatim. Neither document ever tells the
reader to start `network-server` with `--port 7413` — every documented and default
server invocation in both files uses port 7000
(`examples/network-client/src/main.mfb:101`, `:274`; `examples/network-server/src/main.mfb:92`,
`:463`, `:107`). A reader who follows the client's documented command literally,
against a server started with its documented/default command, gets a connection
failure on all three transports because the ports don't match. **Correct
behavior: every command the client's docs show and every command the server's
docs show must reference the same port**, so a reader who copy-pastes the
documented commands in order gets a working connection, not a `Failed` line.

Separately, in the same file, `usage()`'s help text has a copy-paste/ordering bug:
the line `"Defaults to HOST."` (`examples/network-client/src/main.mfb:105`) is
printed directly under the `--allow-self-signed` flag's two-line description
(`:103`-`:104`) instead of under `--server-name`'s one-line description (`:102`),
which is the flag it actually documents (`--server-name`'s default is the `--host`
value; `--allow-self-signed` takes no argument and has no "default"). **Correct
behavior: `--help` output attaches each description line to its own flag.**

References:

- `bugs/completed/bug-477-tls-connect-cannot-accept-a-self-signed-certificate.md:130-136` —
  the session that introduced port 7413: an ad hoc `/tmp/b477-verify.c` OpenSSL
  probe was run "against `examples/network-server --tls certs/cert.pem` on :7413"
  during that bug's Phase-1 measurements. `git log --oneline -S "port 7413" --
  examples/network-client/src/main.mfb examples/network-server/README.md` shows
  the string entered both files in commit `7e36c4ddc` ("bug-477: audit finding,
  docs, and the example that demonstrates the flag"), the same commit that landed
  the `--allow-self-signed` feature — the throwaway probe port from that
  session's manual testing was carried into the committed example instead of the
  feature's actual default.
- `examples/network-server/README.md` — the server's own run instructions, all
  of which use 7000 or state it as the default.

## Failing Reproduction

Built and run today, 2026-09-25:

```
$ target/release/mfb build examples/network-server
Building network-server (executable) for macos-aarch64
Wrote executable to examples/network-server/build/network-server.out

$ target/release/mfb build examples/network-client
Building network-client (executable) for macos-aarch64
Wrote executable to examples/network-client/build/network-client.out

$ cd examples/network-server && ./build/network-server.out --tcp &
Server 47387 (Ctrl-C to stop)
Listening tcp 127.0.0.1:7000
```

Following the client's documented TLS-example command literally (port from
`examples/network-client/src/main.mfb:31`), against the server just started with
its own documented/default command:

```
$ cd examples/network-client && ./build/network-client.out --port 7413
Client 48269
[tcp 127.0.0.1:7413]
Failed 48269 Network operation failed before a connection was established.
[udp 127.0.0.1:7413]
Failed 48269 no reply from 127.0.0.1:7413
[tls 127.0.0.1:7413]
Failed 48269 TLS handshake, certificate validation, SNI validation, or protocol operation failed.
```

- Observed: all three transports fail — the server is listening on 7000, not
  7413, and nothing in either doc told the reader to start it on 7413.
- Expected: the documented client command connects to the documented/default
  server.

Contrast case — the same server, same client binary, correct (default) port:

```
$ ./build/network-client.out --port 7000
Client 48535
[tcp 127.0.0.1:7000]
Connected 48535
Data 48535 Hello 048fb8bb-3654-4b4b-94b2-1b233b901203
Data 48535 Update 048fb8bb-3654-4b4b-94b2-1b233b901203 01
...
Data 48535 Update 048fb8bb-3654-4b4b-94b2-1b233b901203 05
Disconnected 48535
[udp 127.0.0.1:7000]
```

TCP connects and completes its five-update session immediately (UDP then fails
for an unrelated, expected reason — no UDP listener was started; only `--tcp`
was passed to the server — which bounds this bug to the port mismatch, not a
general client/server incompatibility).

The server process (pid 47387) was killed immediately after this capture:
`kill 47387`, confirmed absent from `pgrep -fl network-server.out` afterward
(two unrelated, pre-existing `network-server.out` processes on other ports,
pids 44872 and 45913, were left untouched — they predate this session and are
not part of this reproduction).

Second reproduction, the `usage()` misplacement — built and run today:

```
$ ./build/network-client.out --help
network-client — try TCP, UDP, and TLS against examples/network-server.

USAGE
  network-client [--host HOST] [--port PORT] [--server-name NAME] [--allow-self-signed]

OPTIONS
  --host HOST        Server address. Default 127.0.0.1.
  --port PORT        Server port. Default 7000.
  --server-name NAME Name the TLS certificate must match, sent as SNI.
  --allow-self-signed  Accept a TLS chain whose only fault is an untrusted
                       root. The name and the dates are still checked.
                     Defaults to HOST.
  --help             Show this text and exit.

Every transport is attempted in turn — TCP, UDP, TLS — and each attempt
runs to completion before the next one starts. The client leaves after
the fifth Update line.
```

- Observed: "Defaults to HOST." is printed as if it were part of
  `--allow-self-signed`'s description.
- Expected: "Defaults to HOST." printed directly under `--server-name NAME ...`
  (the flag whose default actually is the `--host` value), with
  `--allow-self-signed`'s two lines standing alone.

## Root Cause

**Port mismatch.** `examples/network-client/src/main.mfb:31` (the TLS-example
line inside the file's header comment block) and
`examples/network-server/README.md:93` (its copy) both hardcode `--port 7413`.
No line in either file — not the client's own header comment, not
`examples/network-server/README.md`'s "Running" section, not
`examples/network-server/src/main.mfb`'s `usage()`
(`examples/network-server/src/main.mfb:92`, `:106`-`:107`) — ever shows or tells
the reader to start the server on 7413; every server-side command and the
compiled-in default (`examples/network-server/src/main.mfb:463`) is 7000. Per
`bugs/completed/bug-477-tls-connect-cannot-accept-a-self-signed-certificate.md:130-136`,
7413 was the ad hoc port used for a one-off OpenSSL probe (`/tmp/b477-verify.c`)
during that bug's manual investigation; `git log --oneline -S "port 7413" --
examples/network-client/src/main.mfb examples/network-server/README.md` shows
it was committed into both files in `7e36c4ddc`, the same commit that landed
`--allow-self-signed` — the session's scratch port leaked into the shipped
example instead of the feature's real default (7000, or any port the doc also
shows starting the server on).

**Misplaced help line.** `examples/network-client/src/main.mfb:usage` prints
each flag's description as a literal `io::print` string per line, in
declaration order, with no structural association between a flag name and its
continuation lines — line 105 (`"Defaults to HOST."`) sits textually below
lines 103-104 (`--allow-self-signed`'s two-line description) and above line 106
(`--help`'s description), so visually it reads as this flag's third line. It
is a leftover/misordered line from when `--server-name`'s description
(`examples/network-client/src/main.mfb:102`, one line, ending mid-sentence with
no default stated) and `--allow-self-signed`'s (`:103`-`:104`) were written or
reordered; the line describing `--server-name`'s default was never moved back
next to it. There is no compiler check over `io::print` string literals, so
nothing catches a help string printed in the wrong position.

## Goal

- `examples/network-client/src/main.mfb`'s header comment and
  `examples/network-server/README.md` reference the same port for the
  paired example commands they show, so copy-pasting them in order produces a
  working connection (checked by re-running the Failing Reproduction's
  contrast case with the doc's own commands).
- `network-client --help` prints "Defaults to HOST." immediately after
  `--server-name NAME`'s description line, not after `--allow-self-signed`'s.

### Non-goals (must NOT change)

- The server's real default port (7000) or its command-line parsing/behavior —
  this is a documentation-only fix, not a change to which port anything binds
  to by default.
- `--allow-self-signed`'s actual behavior or its two existing description
  lines' wording.
- Any other example's or package's use of port 7413
  (`src/codegen/builtins/tls/func_connect.rs:102`'s `mfb man tls connect`
  example, `scripts/man-examples-not-run.txt:32`) — these are self-contained
  documentation examples with no real paired server to mismatch against; see
  Blast Radius.

## Blast Radius

`grep -rn "7413" .` (excluding `.git` and `build/` directories) repo-wide:

- `examples/network-client/src/main.mfb:31` — the bug; fixed by this bug.
- `examples/network-server/README.md:93` — the bug (verbatim copy of the same
  client command); fixed by this bug.
- `bugs/completed/bug-477-tls-connect-cannot-accept-a-self-signed-certificate.md:136,495,502` —
  a closed bug's dated investigation notes describing a manual OpenSSL probe run
  against a manually-started server on :7413; historical record, not live
  guidance — out of scope, not touched.
- `src/codegen/builtins/tls/func_connect.rs:102` — a standalone `mfb man tls
  connect` code example (`tls::connect("127.0.0.1", 7413, ...)`) with no
  corresponding server-start instruction anywhere near it and no claim of
  pairing with `examples/network-server` — unaffected, no server-side doc to
  mismatch.
- `scripts/man-examples-not-run.txt:32` — a skip-list entry noting that man
  example needs a live TLS server on 127.0.0.1:7413 to actually run; consistent
  with the func_connect.rs example above, not with examples/network-client — out
  of scope.
- `tools/math-kernels/ref/*.ref`, `third_party/utf8proc/utf8proc_data.c`,
  `packages/timezones/src/data.mfb`, various golden files, `examples/wind/src/earth.mfb` —
  numeric substrings that happen to contain "7413" (float bit patterns, Unicode
  code-point tables, timezone transition epochs, a float literal); unrelated,
  confirmed by inspection of each grep hit's surrounding context.

`grep -rn "7000" examples/network-client examples/network-server` (excluding
`build/`):

- `examples/network-client/src/main.mfb:8,14,101,274` — all consistent with the
  real default; no change needed.
- `examples/network-server/src/main.mfb:92,107,463` — all consistent with the
  real default; no change needed.

`grep -rn "8443" examples/network-client examples/network-server` (excluding
`build/`): `examples/network-client/src/main.mfb:9` and
`examples/network-server/src/main.mfb:106` both use 8443 only as a generic
*"here's a non-default port syntax"* illustration (`--host 10.0.0.4 --port
8443` on the client side, `--tls certs/cert.pem certs/key.pem --port 8443` on
the server side) — these are not presented as a matched pair the reader is
meant to run against each other (the client example changes `--host` too, so
it is not a "run this against the server's default" case), so they are not the
same failure mode as the 7413 pair. Unaffected — out of scope for this bug.

## Fix Design

Recommended fix, and why: **update the two 7413 references to say 7000**, not
add a "start the server with --port 7413" instruction. Reasoning:

1. Every other example in both files already uses 7000 (the compiled-in
   default); making the TLS example consistent with them costs one number
   change in each of two files and needs no new sentence explaining why this
   one example alone requires a non-default server invocation.
2. The TLS example's point (per the surrounding comment,
   `examples/network-client/src/main.mfb:24-35`) is `--allow-self-signed`, not
   port selection — a nonstandard port adds nothing to that demonstration and
   is pure incidental noise the reader has to reconcile.
3. Documenting "start the server with --port 7413 to try this" would require
   *also* correcting the paired `examples/network-server/README.md` TLS
   run line (`./build/network-server.out --tls certs/cert.pem certs/key.pem`,
   README.md's own "Running" section) to add `--port 7413` there too, which is
   strictly more doc surface changed for a worse (still port-nonstandard)
   result.

Rejected alternative: leave the port at 7413 and add a companion
`--port 7413` to the server's documented TLS start command. Rejected because it
multiplies the doc surface touched (both files' TLS command *and* the reader
now needs an explanation of why TLS alone uses a different port) for no
benefit over simply matching the existing default used everywhere else in both
files.

For `usage()`: move the line `io::print("                     Defaults to HOST.")`
from immediately after the `--allow-self-signed` description
(`examples/network-client/src/main.mfb:103`-`:105`) to immediately after the
`--server-name NAME` line (`examples/network-client/src/main.mfb:102`), so the
argument-list order becomes `--server-name` line + its "Defaults to HOST."
continuation, then `--allow-self-signed`'s two lines standing alone. No wording
changes, no other flags reordered.

## Phases

### Phase 1 — failing repro + audit (no behavior change)

- [x] Build server and client, reproduce the port-mismatch failure and its
      port-7000 contrast case; capture verbatim output (see Failing
      Reproduction — done above, 2026-09-25).
- [x] Build client, capture verbatim `--help` output showing the misplaced
      line (see Failing Reproduction — done above, 2026-09-25).
- [x] Complete the repo-wide `7413`/`7000`/`8443` blast-radius grep and record
      a verdict per hit (see Blast Radius — done above, 2026-09-25).

Acceptance: reproduction captured verbatim above; blast-radius list complete
with a verdict per site.
Commit: — (this document only; no code touched yet)

### Phase 2 — the fix

- [x] `examples/network-client/src/main.mfb:31` — change `--port 7413` to
      `--port 7000` (or drop the flag entirely, since 7000 is the default —
      pick whichever reads more naturally in context; either satisfies the
      Goal). (Dropped the flag entirely.)
- [x] `examples/network-server/README.md:93` — same change, keeping the two
      commands identical to each other as they are today. (Same: dropped the
      flag; both commands remain identical to each other.)
- [x] `examples/network-client/src/main.mfb:103`-`:105` — move the
      `"Defaults to HOST."` print line to directly follow the
      `--server-name NAME` line (`:102`).

Acceptance: re-running the Failing Reproduction's client command (now without
a port mismatch) connects on the first (TCP) transport against a server
started with its documented command; `--help` output shows "Defaults to
HOST." directly under `--server-name`.
Commit: 031c2cd7d

### Phase 3 — validation

- [x] `scripts/build-examples.sh` (or the relevant subset covering
      network-client/network-server) — confirm both still build clean.
      (Both built clean on macos-aarch64 directly; the full multi-target run
      already covered network-client/network-server in this lane's bug-704
      and bug-705 work on this same worktree, after this doc's own source
      changes.)
- [x] Re-run the full reproduction end-to-end: start the server with its
      documented command, run the client with the doc's (now-corrected) TLS
      example command, confirm TCP connects (TLS still fails against the
      self-signed cert as documented/expected — that failure is the example's
      intended lesson and is unrelated to the port fix). Kill the server
      afterward. (Confirmed: TCP transport connects and receives Update
      lines; server killed after.)
- [x] Re-run `--help` and confirm the line order. (Confirmed: "Defaults to
      HOST." now prints directly under `--server-name NAME`.)

Acceptance: build-examples passes; the doc's own commands work end to end;
`--help` text is corrected; no server process left running afterward.
Commit: 031c2cd7d (no additional commit; validation only, no further code
change)

## Validation Plan

- Regression test(s): none exist for printed help/comment text in this
  project's test suite (docs-only artifacts, not compiled or asserted by
  `cargo test`); the check is the manual re-run in Phase 3.
- Runtime proof: Phase 3's end-to-end re-run (build both, start server, run
  client with the doc's exact command, observe TCP `Connected`/`Data`/
  `Disconnected` instead of `Failed`; observe corrected `--help` text).
- Doc sync: this bug's two touched files ARE the docs; no spec or man page
  references either project's example port.
- Full suite: `scripts/build-examples.sh` (examples build gate); no `cargo
  test` target covers example-project prose.

## Summary

This is a pure documentation drift bug: a scratch port from a prior bug's
manual test session (bug-477, port 7413) was committed into two files'
checked-in example text in commit `7e36c4ddc` and never reconciled with the
feature's real default (7000), which every other example in both files
already uses correctly. The `usage()` line-ordering issue is a small,
independent copy-paste slip in the same file. Both are one-line-per-site
text fixes with no behavioral, ABI, or default-value change; the only real
risk is missing a paired reference (mitigated by the repo-wide grep in Blast
Radius) or introducing a new inconsistency between the client's comment and
the server's README, which Phase 3's end-to-end re-run catches directly.
