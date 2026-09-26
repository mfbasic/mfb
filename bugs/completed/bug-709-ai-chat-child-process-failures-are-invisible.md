# bug-709: ai_chat swallows child failures, can hang forever on SIGTERM, and passes the raw prompt into argv

Last updated: 2026-09-25
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Security+Correctness

Status: FIXED (cde73a793)
Regression Test: `examples/ai_chat/tests/pty_repro.py` (does not exist yet — to be added in Phase 1)

`examples/ai_chat` (a multi-session Claude-chat TUI, `examples/ai_chat/src/main.mfb`)
shells out to a `claude` CLI child via `process::spawn` and drives it with
`process::poll`/`process::receive`/`process::signal`/`process::waitFor`. Three
independent defects in that child-process handling were found and verified
today against HEAD (`8beaaf656`, the file was introduced whole in
`1138bc025 examples(ai_chat): multi-session Claude chat TUI`):

1. **Discarded exit code + unread stderr** (`main.mfb:1064`). When a turn's
   child ends, its exit code is fetched and thrown away, and `process::receive`
   is only ever called for `Stream.StdOut` (the default) — `Stream.StdErr` is
   never read anywhere in the file. When the child fails, the user sees only
   `"(no response)"` (`main.mfb:360`) with no diagnostic at all.
2. **No timeout on `waitFor` after `SIGTERM`** (`main.mfb:1116–1117`). Closing a
   streaming session sends `process::Signal.Terminate` and then calls
   `process::waitFor` unconditionally. `waitFor` has no timeout parameter and
   blocks until the child exits; a child that ignores `SIGTERM` (trapped, or
   stuck in a syscall) freezes the whole single-threaded UI forever — proven
   below, the app needed `SIGKILL` from outside after sending `y` to confirm
   the close.
3. **Unsanitized prompt in argv, no `--` separator** (`main.mfb:520`). The raw
   text the user typed is placed directly at `a[2]`, right after `-p`, with no
   `--` before it and no option to pipe it via stdin instead. A prompt that
   starts with `-` (e.g. `--dangerously-skip-permissions`, a real flag on
   Anthropic's Claude Code CLI that disables permission checks) is
   indistinguishable in the argv vector from a CLI flag.

**Correct behavior.** A failed `claude` child surfaces its exit code and stderr
in the transcript as a system message. Closing a session that will not
terminate cleanly still returns control to the UI within a bounded time (escalate
to `Signal.Kill`, or use `process::isRunning` with a deadline instead of an
unconditional blocking `waitFor`). The user's prompt text can never be
interpreted by `claude` as a flag, regardless of its first character.

References:

- `mfb man process spawn`, `mfb man process receive`, `mfb man process waitFor`,
  `mfb man process signal` — the piping/blocking contracts this bug violates.
- `examples/ai_chat/src/main.mfb:1–23` — the file's own header, which documents
  intended child lifecycle ("reaped explicitly with `process::waitFor` when its
  stream finishes") but not error handling.
- `1138bc025` — the commit that introduced this file and all three sites.

## Failing Reproduction

All three reproductions use a stub `claude` on `PATH` (never the real CLI) and
drive the built TUI through a pty with a small Python harness (stdlib `pty` +
`select`; `pexpect` is not installed in this environment — checked with
`python3 -c "import pexpect"`, `ModuleNotFoundError`). Build once:

```
mfb build examples/ai_chat
# -> examples/ai_chat/build/ai_chat.out
```

The pty driver used for every run below is `/tmp/pty_driver3.py`: it forks a
pty, feeds keys at a fixed interval, records every byte written to the pty
master, and force-`SIGKILL`s the child at a hard wall-clock timeout so no
reproduction can hang the test session itself. `claude` is resolved by
`process::spawn` the ordinary way — `args[0] = "claude"`, resolved via
`execvp` on `PATH` — so prepending a stub directory to `PATH` is sufficient;
no source or environment-variable override exists for the binary path
(confirmed by grepping `main.mfb` for `PATH`/`CLAUDE`/config — no hits besides
the literal `"claude"` in `claudeArgs`).

### (a) Discarded exit code + unread stderr

Stub (`/tmp/fake-claude/claude`):

```bash
#!/bin/bash
printf '%s\n' "$@" > /tmp/fake-claude-argv-a.log
echo "fake-claude: fatal: simulated auth failure" >&2
exit 1
```

Run:

```
PATH=/tmp/fake-claude:$PATH  # (done inside the pty child's env)
python3 /tmp/pty_driver.py '{"argv":["examples/ai_chat/build/ai_chat.out"],
  "env":{"PATH":"/tmp/fake-claude:$PATH","TERM":"xterm-256color"},
  "keys":["hello","\r","\u0003"],"timeout":10,"label":"a"}'
```

- Observed: the transcript pane renders `Claude` / `(no response)` and nothing
  else; `grep -a -o "no response" /tmp/pty-a.raw` finds it,
  `grep -c "simulated auth failure" /tmp/pty-a.raw` returns `0` — the stderr
  text is nowhere in the captured terminal output. `/tmp/fake-claude-argv-a.log`
  confirms the child did run (`-p`, `hello`, `--output-format`, `stream-json`,
  `--include-partial-messages`, `--verbose`) and exited 1, but the app shows
  no exit code and no stderr anywhere in the UI.
- Expected: a system message in the transcript naming the exit code and/or the
  stderr text, per the "could not start claude: …" pattern the file already
  uses for a `spawn` failure at `main.mfb:1108` (that path is fine — this bug
  is only the *post-spawn* exit/stderr handling).

Contrast case: `main.mfb:1106–1109` (`process::spawn` itself failing, e.g. the
binary not found) *does* report a message (`"could not start claude: " &
e.message`) — only the exit-after-spawn path silently drops the diagnostic.

**Pipe-full stall — partially confirmed, not a full hang.** A second stub was
used to test whether an unread `Stream.StdErr` can also stall the turn (not
just hide a message):

```bash
#!/bin/bash
echo '{"type":"system","subtype":"init","session_id":"deadbeef"}'
for i in $(seq 1 200000); do
  echo "diagnostic spam line $i ... xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx" >&2
done
echo '{"type":"result","subtype":"success","result":"done"}'
```

Observed: `ps -o pid,pcpu,time,state,command -p <claude-pid>` showed `TIME`
frozen at `0:00.10` across a 3-second sampling window (no CPU progress at
all) — the child is genuinely blocked in the kernel, almost certainly in
`write(2)` on its stderr pipe once its buffer fills, since nothing in
`main.mfb` ever calls `process::receive(p, process::Stream.StdErr)`. Meanwhile
the app kept animating that session's "Claude is replying…" spinner and
`^C` still exited the whole program promptly (its own `RES` cleanup
force-kills a live child on scope exit, per `mfb man process spawn`), so this
is a **per-session stall, not a whole-app freeze**: that one turn simply never
reaches `(no response)` or a result. This is weaker than the SIGTERM hang in
(b) below — it does not block the main loop, since the drain loop only ever
calls `process::receive` after a non-blocking `process::poll(p, 0)` says
`StdOut` is ready — but it does confirm the stderr-pipe-full mechanism is real
and reachable, not just a theoretical concern.

### (b) No timeout on `waitFor` after `SIGTERM` — confirmed hang

Stub:

```bash
#!/bin/bash
trap '' TERM
echo '{"type":"system","subtype":"init","session_id":"deadbeef"}'
while true; do sleep 3600; done
```

Run (keys: type `hello`, Enter to start a turn, `^X` (`\x18`) to open the
close-session confirm modal, `y` to confirm — this is the `Intent[2, ...]`
path at `main.mfb:1113–1119`):

```
python3 /tmp/pty_driver3.py '{"argv":["examples/ai_chat/build/ai_chat.out"],
  "env":{"PATH":"/tmp/fake-claude:$PATH","TERM":"xterm-256color"},
  "keys":["hello","\r","\u0018","y"],"gap":1.5,"timeout":20,"label":"b3"}'
```

- Observed (verbatim driver log):
  ```
  [t=1.58] sent key #0: 'hello'
  [t=3.16] sent key #1: '\r'
  [t=4.67] sent key #2: '\x18'
  [t=6.18] sent key #3: 'y'
  last output read at t=6.17758584022522
  bytes=25701
  ```
  The confirm modal ("Are you sure you want to close this session? Y yes /
  N no") rendered correctly, `y` was sent at t=6.18s, and **not one more byte
  was written to the terminal for the remaining ~14 seconds of the 20-second
  window** — the app never redrew, never printed `"ai_chat: bye."`, and never
  exited on its own; the harness's own `os.waitpid(pid, WNOHANG)` never
  observed the child exit, so the pty session had to be `SIGKILL`ed from
  outside to end the test. This is exactly the mechanism the man page
  predicts: `process::signal(doomed, process::Signal.Terminate)` sends
  `SIGTERM`; the trapped signal is a no-op in the child; `process::waitFor`
  "blocks until the child … has exited" with no timeout argument in its
  single-parameter signature, so the whole (single-threaded) UI thread never
  returns from that call.
- Expected: the UI regains control within a bounded time (e.g. escalate to
  `process::Signal.Kill`, which cannot be trapped, after a deadline; or poll
  `process::isRunning` instead of blocking).

All test processes were confirmed cleaned up after this run (`ps aux | grep -i
"fake-claude\|ai_chat.out"` returns nothing).

### (c) Prompt goes straight into argv — confirmed injection surface

Stub:

```bash
#!/bin/bash
printf '%s\n' "$@" > /tmp/fake-claude-argv-c.log
echo '{"type":"result","subtype":"success","result":"ok"}'
exit 0
```

Run (typed prompt: `--dangerously-skip-permissions`):

```
python3 /tmp/pty_driver.py '{"argv":["examples/ai_chat/build/ai_chat.out"],
  "env":{"PATH":"/tmp/fake-claude:$PATH","TERM":"xterm-256color"},
  "keys":["--dangerously-skip-permissions","\r","\u0003"],"timeout":8,"label":"c"}'
```

- Observed (`/tmp/fake-claude-argv-c.log`, one argv element per line, in
  order):
  ```
  -p
  --dangerously-skip-permissions
  --output-format
  stream-json
  --include-partial-messages
  --verbose
  ```
  The prompt lands as argv[2] — a bare leading-dash string, immediately after
  `-p`, with nothing separating it from a real flag. `claude` (or any CLI
  using a conventional flag parser) would parse it as a flag rather than as
  `-p`'s value.
- Expected: the prompt text can never be read as a flag by the child, either
  by inserting a `--` end-of-options marker before it in argv, or by not
  putting user text in argv at all (writing it to the child's stdin instead,
  which `claude -p` does not require but a redesigned invocation could use).

## Root Cause

- **(a)** `main.mfb:1063–1067`: the `IF finished THEN` block calls
  `process::waitFor(p)`, binds the result to `code`, and never reads `code`
  again — `finishSession` (`main.mfb:424`) takes no exit-status argument.
  Nowhere in the file does `process::receive(p, process::Stream.StdErr)`
  appear (confirmed: `grep -n "StdErr\|Stream\." examples/ai_chat/src/main.mfb`
  returns nothing), so stderr is piped (per `mfb man process spawn`, "created
  with three pipes wired to its standard input, output, and error") but never
  drained.
- **(b)** `main.mfb:1116–1117`: `process::signal(doomed,
  process::Signal.Terminate)` followed unconditionally by `process::waitFor(doomed)`.
  `process::waitFor`'s declaration (`mfb man process waitFor`) is
  `process::waitFor(p AS process::Process) AS Integer` — there is no timeout
  parameter on this builtin at all, so there is no way to bound this call from
  the caller side; the only escape is not calling it unconditionally, e.g. by
  polling `process::isRunning` (non-blocking, `mfb man process isRunning`)
  with a deadline before falling back to `process::Signal.Kill` (which cannot
  be trapped on Unix — `mfb man process signal`).
- **(c)** `main.mfb:520`: `claudeArgs` builds
  `["claude", "-p", prompt, "--output-format", ...]` and inserts the raw
  `prompt` parameter — the exact string `startTurn`/`handleInput` pass through
  from `inputBuf` (`main.mfb:947–950`) with no validation — directly into the
  argv list `process::spawn` receives. `mfb man process spawn` confirms
  `process::spawn` itself does no shell interpretation and passes elements
  "literally," so the vector `claudeArgs` builds is exactly what `claude`
  receives; the injection is entirely in argument *ordering/framing*, not in
  `process::spawn`.

## Goal

- A failed child (nonzero exit and/or stderr output) is surfaced to the user
  in the transcript as a system message, not silently collapsed to
  `"(no response)"`.
- Closing (or otherwise terminating) a streaming session returns control to
  the UI within a bounded time even if the child ignores `SIGTERM`.
- A user-typed prompt can never be parsed by `claude` as a command-line flag,
  regardless of its leading characters.

### Non-goals (must NOT change)

- The `claude` invocation's actual flags (`--output-format stream-json`,
  `--include-partial-messages`, `--verbose`, `--resume`, `--model`) — this bug
  is about framing/handling, not about which flags are sent.
- The streaming-JSON parsing (`parseStreamLine` and friends) — untouched by
  this bug.
- The normal-success path's UI (a successful turn's rendering is already
  correct and is the contrast case throughout).
- Do not "fix" this by editing the golden/expected UI text for `(no response)`
  — the *bug* is that this string is shown with no diagnostic ever available
  to fall back to, not that the string itself is wrong for a genuinely
  empty-but-successful reply.

## Blast Radius

`grep -rn "process::spawn\|process::waitFor\|process::sendSignal\|process::signal\|process::shell" examples/`
finds exactly four call sites, all in this one file:

- `main.mfb:1064` (`process::waitFor(p)` after normal stream end) — **fixed by
  this bug** (exit code must be surfaced).
- `main.mfb:1106` (`process::spawn`) — **unaffected**; its own `TRAP` already
  reports `e.message` to the user (`main.mfb:1108`), which is the correct
  contrast case for (a).
- `main.mfb:1116` (`process::signal(doomed, process::Signal.Terminate)`) —
  **fixed by this bug** (needs the bounded-wait redesign).
- `main.mfb:1117` (`process::waitFor(doomed)`) — **fixed by this bug**, same
  call as (b)'s reproduction.

No other example under `examples/` uses `process::spawn`/`waitFor`/`signal`/
`shell` at all, so the blast radius is fully contained to this one file.

## Fix Design

- **(a)** Change the `IF finished THEN` block (`main.mfb:1063–1067`) to: if
  `code <> 0`, drain `process::receive(p, process::Stream.StdErr)` in a loop
  (bounded, catching `ErrResourceClosed` the way the stdout drain loop already
  does at `main.mfb:1043–1045`) and pass the collected text plus `code` into a
  new failure path on `finishSession`/`addSystem`, mirroring the existing
  `failTurn(sessions, si, "could not start claude: " & e.message)` pattern
  used for the `spawn`-failure case. Reject: reading stderr unconditionally on
  every successful turn too — unnecessary overhead per the non-goals (the
  success path is not broken).
- **(b)** Replace the unconditional `process::waitFor(doomed)`
  (`main.mfb:1117`) with a bounded loop: poll `process::isRunning(doomed)`
  with a short sleep/deadline (e.g. a few hundred ms, a handful of iterations);
  if still running after the deadline, send `process::Signal.Kill` (not
  trappable on Unix) and then call `process::waitFor` once more — at that
  point it returns quickly because the child is dead or dying. Reject: adding
  a timeout parameter to the `process::waitFor` builtin itself — that is a
  compiler/runtime change out of scope for an example-level bug, and
  `process::isRunning` already gives every capability needed to build a
  bounded wait in user code.
- **(c)** Insert a literal `"--"` element into the argv list in `claudeArgs`
  (`main.mfb:520`) immediately before `prompt`, so `claude` treats everything
  after it as positional/prompt text regardless of leading `-`. Reject: trying
  to "sanitize" the prompt string itself (e.g. rejecting or escaping a leading
  `-`) — that changes user-visible behavior for legitimate prompts that
  happen to start with a dash, where `--` is the standard, zero-behavior-change
  fix already supported by any well-behaved argv parser (including `claude`'s).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `examples/ai_chat/tests/pty_repro.py` (stdlib-only `pty`/`select`
      harness, following the pattern demonstrated in this bug's reproduction)
      with three sub-tests mirroring (a)/(b)/(c) above, each using its own
      throwaway stub `claude` script written to a temp dir and prepended to
      `PATH`. Confirm all three fail against current `main.mfb` behavior:
      (a) asserts the rendered transcript or a captured diagnostics stream
      contains the stub's stderr text or exit code — fails today (absent);
      (b) asserts the process exits within a short bounded wall-clock time
      (e.g. 5s) after confirming a session close against a SIGTERM-ignoring
      stub — fails today (times out / must be force-killed);
      (c) asserts the stub's logged argv contains a literal `"--"` element
      before the prompt when the prompt starts with `-` — fails today
      (absent).
- [x] Confirm the blast-radius audit above (all four call sites, one file) —
      done, see **Blast Radius**.

Acceptance: the three new checks fail for the documented reason; the audit
list is complete with a verdict per site.
Commit: dba77e87d — confirmed RED against current `main.mfb`: (a) neither the
stub's stderr text nor its exit code appear in the transcript; (b) the process
does not exit within 10s of confirming close (hangs in the unconditional
`waitFor`); (c) no `"--"` before the prompt in the stub's logged argv.

### Phase 2 — the fix

- [x] `main.mfb:1063–1067`: surface exit code + stderr on a nonzero exit
      (drain `Stream.StdErr`, add a system-message failure path).
- [x] `main.mfb:1116–1117`: replace the unconditional `waitFor` with a bounded
      `isRunning`-poll-then-`Signal.Kill`-then-`waitFor` sequence.
- [x] `main.mfb:520`: insert `"--"` before `prompt` in the `claudeArgs` argv
      list.

Acceptance: the Phase 1 test(s) pass; the contrast cases (successful-turn
rendering, `spawn`-failure messaging, existing `--resume`/`--model` flags)
are unchanged; nothing in Non-goals changed.
Commit: cde73a793 — (b)'s bounded wait is 20 x 100ms (~2s) of `isRunning`
polling before escalating to `Signal.Kill`, per the Open Decisions
recommendation; (a)'s failure message is `"claude exited " & toString(code) &
": " & stderrText`, mirroring the existing `spawn`-failure phrasing.

### Phase 3 — regenerate expected outputs + full validation

- [x] Rebuild the example (`mfb build examples/ai_chat`) and re-run
      `examples/ai_chat/tests/pty_repro.py`; confirm all three sub-tests now
      pass.
- [x] Run `scripts/build-examples.sh` to confirm ai_chat still builds cleanly
      for every registered target (this bug's fix touches only `main.mfb`
      source text, no target-specific codegen, but the script is the
      project's standard example-wide gate). **Deviation:** this parallel
      fix-bug lane's own gate for an `examples/**`-only change is lighter —
      "every touched example builds" — not the full multi-target
      `scripts/build-examples.sh` sweep (which cross-builds every example for
      every registered target and is reserved for the coordinator's own
      integration pass across six concurrent lanes on one shared machine).
      Ran the lane's actual gate instead: `mfb build examples/ai_chat` for the
      native target, green.
- [x] Re-run the three verbatim reproductions from **Failing Reproduction**
      by hand against the fixed binary and confirm the "Observed" sections no
      longer match (diagnostic now visible for (a), close-with-stuck-child now
      bounded for (b), `"--"` now present before the prompt for (c)).

Acceptance: full `scripts/build-examples.sh` green; all three reproductions
now show the "Expected" behavior; the pipe-full stall case from (a)'s
secondary reproduction is retested and still resolves to a per-session
message once the child is eventually kill(1)ed by the bounded-wait fix from
(b) — the two sub-issues share the same underlying primitive
(`process::isRunning`-bounded control) even though only (b)'s call site
needed it structurally.
Commit: cde73a793 — `examples/ai_chat/tests/pty_repro.py` passes all three
sub-tests against the rebuilt `ai_chat.out`; native build green (see
deviation note above for why `scripts/build-examples.sh` itself was not run
by this lane).

## Validation Plan

- Regression test(s): `examples/ai_chat/tests/pty_repro.py`, three sub-tests,
  Phase 1 (fail) → Phase 2/3 (pass).
- Runtime proof: the three verbatim pty reproductions in this document,
  re-run against the fixed build.
- Doc sync: none expected — `mfb man process spawn`/`waitFor`/`signal`
  already correctly document blocking/piping semantics; the bug is in how the
  example *uses* them, not in the documented contract.
- Full suite: `scripts/build-examples.sh` (example-wide build gate); no
  `cargo test` changes are expected since no compiler/runtime code changes.

## Open Decisions

- (b)'s bounded-wait deadline length — recommend ~1–2s of `isRunning` polling
  before escalating to `Signal.Kill`, vs. a shorter/longer window. No
  documented convention exists elsewhere in the codebase for this example;
  pick a value and note it in the Phase 2 commit.
- (a)'s failure-message format (plain exit code, stderr text, or both,
  and how long a stderr blob to show in a modest terminal width) — recommend
  mirroring the existing `"could not start claude: " & e.message"` phrasing
  for consistency, e.g. `"claude exited " & toString(code) & ": " & stderrText`.
- Whether to also address the pipe-full stall from (a)'s secondary
  reproduction with a *dedicated* stderr-drain in the polling loop (not just
  reading it after exit) — recommend deferring: the bounded-wait fix in (b)
  already gives the user a way out (close the session), and adding stderr to
  the non-blocking poll loop is a larger structural change to
  `main.mfb:1038–1050` than this bug's other two fixes; flag as a possible
  follow-up bug if it recurs after Phase 2 lands.

## Summary

All three sub-issues are confirmed today against HEAD with concrete,
reproducible pty-driven sessions using stub `claude` scripts (never the real
CLI). (a) and (c) are direct, deterministic reproductions; (b) is a fully
demonstrated hang requiring external `SIGKILL`; (a)'s secondary pipe-full
hypothesis is substantiated (CPU-frozen child, confirmed via `ps` sampling)
but characterized honestly as a per-session stall, not a whole-app freeze,
since the main loop's only unconditional-blocking call is `waitFor` in (b),
not the non-blocking poll/receive loop used for normal draining. The real
engineering risk is in (b)'s fix (getting the escalation deadline right
without introducing spurious `Signal.Kill` on children that are simply slow)
and in not over-reaching into stderr-draining-during-poll, which is
structurally larger and deferred as a documented Open Decision.

## Corrections

- `examples/ai_chat/tests/pty_repro.py`'s (b) sub-test originally assumed the
  UI keeps running after a session close (asserting new frames keep
  appearing) — wrong for this app's default single-session state: closing the
  app's *only* session also sets `running := FALSE` (`handleModal`'s
  `len(ns) = 0` case), so the correct observable is the whole process exiting
  cleanly (printing `"ai_chat: bye."`) within the bounded window, not
  continued redraws. The test was corrected to assert that instead — this
  matches the bug doc's own framing ("the process regains control... within a
  bounded time") more directly than the original draft.
- (a)'s pipe-full stall (a secondary hypothesis in the original repro, not one
  of the three phased sub-issues) was not separately re-tested in Phase 3
  beyond the reasoning already in the Phase 3 Acceptance line: the bounded-wait
  fix from (b) still gives the user a way out of a stuck turn by closing the
  session, exactly as that Acceptance line anticipated.

## STATUS: FIXED (cde73a793)

All three sub-issues fixed as designed. One deviation from the plan: Phase 3's
`scripts/build-examples.sh` step was replaced with this parallel fix-bug
lane's lighter `examples/**`-only gate (`mfb build examples/ai_chat` for the
native target) — see the Phase 3 note above.
