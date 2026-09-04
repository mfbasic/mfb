# audit-3 — Surface 4: OS-touching runtime packages (fs / net / http / process / thread / term / io / app)

Part of `planning/goal-08-platform-security-review.md`. Finding prefix `OS-`
(fs/process/term/app: OS-01..27; net/http: OS-50..59). Untrusted party: a remote
net/http peer; attacker-controlled paths/filenames/environment; hostile terminal
content; window-system input.

**Verdict: 1 CRITICAL · 8 HIGH · 12 MEDIUM · rest LOW/NTH.** The CRITICAL
(OS-50) is a remote peer-controlled memory disclosure at the socket-write boundary,
**reproduced live by the lead** (root cause of open bug-476). The HIGHs span fd
inheritance, an environment-driven fork-child hang, HTTP request smuggling / CRLF
injection, and HTTP server DoS. Bug docs filed for the CRITICAL and the HIGH
clusters.

## CRITICAL

### OS-50 — `tcp/tls/udp` write of a `String`-returning call selects the byte-list lowering → remote peer-controlled OOB read → **bug-497**

`tcp::write`/`tls::write`/`udp::send` pick the code form by the payload's *static*
type and fail **open**: `static_type_name` returns `None` for any user-function
call, so a `String` from a call is read as a collection block — length = the
string's first 8 bytes, data = 40 bytes in (`builder_values.rs:2419`,
`builder_value_semantics.rs:1113`). Lead-reproduced live: a 22-byte request with a
length field of 1024 returned 1024 bytes of process memory (live strings + heap);
first 8 bytes = 1e6 → 65 536 bytes. Root cause of open bug-476.
Spike: `spikes/audit-3/OS-50/`.

## HIGH

- **OS-01 + OS-04 — spawned children inherit every open fd/socket → bug-499.**
  `fs::open` and the socket helpers omit `O_CLOEXEC`/`SOCK_CLOEXEC`
  (`gen_open.rs:32`, `socket/shared.rs:141`); Windows `CreateProcessA` uses
  `bInheritHandles=TRUE` (`gen_windows.rs:1028`). Lead code-verified the flag/handle
  sites. A spawned helper receives the parent's secrets and sockets.
- **OS-02 — env-replace clear loop never terminates on an `environ` entry with no
  `=` (or leading `=`) → bug-500.** `gen_unix.rs:315-325` reloads `environ[0]` and
  restarts; if `unsetenv(name)` removes nothing the loop spins, allocating an
  unfreed buffer each pass (17 GB/19 s). Lead code-verified the loop.
- **OS-51/52/56 — HTTP server DoS cluster → bug-507.** One malformed chunk-size
  aborts the process (untrapped raise, `func_handle_request.rs:133`); no read
  timeout (slowloris); quadratic head rescan + no caps (2 MiB→0.7 s, 64 MiB≈12 min).
- **OS-53/54/55 — HTTP framing / CRLF injection cluster → bug-506.** Client CRLF via
  the `method` (`helper_normalize_method.rs` rejects only `""`/space, lead-verified);
  server response splitting (`helper_serialize_head.rs` interpolates key/value/reason
  raw, lead-verified); request-smuggling toolbox (duplicate CL last-wins, WS-before-
  `:`, obs-fold, substring `chunked`, body not truncated).
- **OS-23 — `thread::send`/`emit` allocates on the peer thread's arena unlocked →
  bug-498 (MEM-70).** Cross-referenced from Surface 3; lead-reproduced SIGSEGV.

## MEDIUM

- **OS-03** — Windows `spawn`/`shell` resolve a bare name via the current directory
  (binary planting), contra the in-code comment (`gen_windows.rs:1041`).
- **OS-05** — Windows `spawn` of a `.bat`/`.cmd` re-parses argv through cmd.exe
  despite "no shell" (BatBadBut shape) (`func_spawn.rs:30-44`).
- **OS-06** — `term::on` not re-entrant → `term::off` restores raw as cooked; shell
  left `-echo -icanon` (reproduced) (`term/core/term.rs:324`).
- **OS-07** — uncaught-error banner writes the error String raw to fd 2; canvas
  messages interpolate an attacker-named path → ANSI/OSC-52 injection
  (`engine/function/entry.rs:692`, `canvas/func_load_image.rs:62`).
- **OS-08** — `io::print` in TUI mode stamps C0 bytes into cells; presenter emits
  them verbatim (`term/grid/term_grid.rs:658-664`).
- **OS-09** — `MFB_WINAPP_INPUT` env-keyed keystroke injection + unchecked
  `GetEnvironmentVariableW` return → OOB read of a 512-byte static
  (`target/win_x86_64/app/mod.rs:654-696`).
- **OS-10** — `fs::openWithin`'s `openat2` ENOSYS fallback silently degrades
  confinement to final-component-only (`gen_open.rs:793-800`). Residual of audit-2
  OS-04 (bug largely fixed by `openat2`/`O_NOFOLLOW_ANY`).
- **OS-11** — `io::readLine` grows uncapped (contrast `SPILL_MAX_CAPACITY`)
  (`gen_read_line_family.rs:284-303`).
- **OS-12** — `^Z`/`^\` and fault signals uncaught → tty left cbreak/noecho
  (`engine/function/entry.rs:345`).
- **OS-13** — GTK `app::setMode` never runs `TERM_INIT` → 0×0 grid → `memmove` with
  `SIZE_MAX` (`target/linux_gtk/bootstrap.rs:330`).
- **OS-24** — detached `thread::` workers never joined while `_mfb_shutdown`
  `munmap`s their arena (`os/process/process_lifecycle.rs:82-85`).
- **OS-25** — pending-free list drains only on a read of the same queue — unbounded
  when the destination is dead (`runtime/thread/runtime_helpers_thread.rs:1275`).
- **OS-56/57/58/59** — net/http: quadratic head rescan (folded into bug-507);
  `respondPath` existence oracle + `%00` content-type confusion
  (`func_respond_path.rs:147`); multipart `filename` traversal/NUL unsanitized
  (`helper_disposition_param.rs:23`); `net::toUrl` treats `\` as non-separator →
  `http://good.com\@evil.com/` resolves to `evil.com` (`net/helper_authority_end.rs:13`).

## LOW / NTH

OS-14 (dirs 0o755 while files 0o600 — bug-184 fixed only the file half), OS-15
(Windows `WM_CHAR` low-byte mask → ill-formed UTF-8), OS-16/17 (GTK write desync /
prologue off-by-16), OS-18/19 (unclamped tty dims / lead-byte-only control filter),
OS-26 (thread helpers lack their own null guard), OS-20/21/22/27 (NTH).

## Re-verified

audit-1 OS-01 (0o666) FIXED (bug-184; `-rw-------`). "`term::on` leaves ISIG on"
confirmed (masks: macOS `0x108`, Linux `0xA`, ISIG untouched). "reset SIG_DFL
between fork and exec" confirmed (SIGPIPE is the only ignored signal; child
restores it). No gzip/inflate/cookies in `http` (0 grep hits); client follows no
redirects (no SSRF amplification); UDP receive sound (`maxBytes+1` bound);
`tcp::read` capped at 1 MiB (bug-261).

## Bug docs filed

bug-497 (OS-50, CRITICAL), bug-499 (OS-01/04), bug-500 (OS-02), bug-506
(OS-53/54/55), bug-507 (OS-51/52/56). OS-23 → bug-498 (Surface 3).

## Coverage

Read: `builtins/{fs,os,io,process,thread,term,net,tcp,udp,http}/**` (path/env/
spawn/signal/CLOEXEC, the http parse/serialize/limits helpers, the socket layer),
`term/**`, `target/linux_gtk/**`, `target/win_x86_64/app/**`, `os/{process,socket,
syscall}/**`.

Gaps: `builtins/fs/gen_path_builder.rs` (936 lines, `pathNormalize`'s `..` popping)
not read — a traversal-primitive worth a follow-up; `gen_windows.rs:530-940`
env-block size-vs-copy arithmetic not checked (a mismatch would be a heap overflow);
`socket/poll.rs`, `net/gen_ping.rs`/`func_lookup.rs`, most `tcp/udp` descriptors,
and the `tls` package (own surface) skimmed. Every repro is macOS-aarch64; OS-50
lives in arch-neutral code but was not re-run on Linux/Windows.
