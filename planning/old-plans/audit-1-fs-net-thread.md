# Audit 1 — Filesystem / Network / Thread Runtime

Code-grounded security review of the MFBASIC runtime's OS-interaction helpers.
The helpers are AArch64/x86_64 code emitters (`lower_*_helper`) in
`src/target/shared/code/` plus the per-platform syscall/errno tables in
`src/target/{macos,linux}_*/code.rs`. The "logic" of each helper is the exact
sequence of syscalls / libc calls and flags it emits, so findings cite the
emit sites and the decoded flag/mode constants.

Scope reviewed: `fs_helpers*.rs`, `net/{mod,io,poll}.rs`, `tls/{mod,openssl}.rs`,
`runtime_helpers_thread.rs`, `builder_arena_transfer.rs`, the `*_specs.rs`
contracts, and `{macos,linux}_aarch64/code.rs` flag tables. All flag/mode
constants below were decoded and confirmed numerically.

Severity summary:
- HIGH: 2 (OS-01 world-writable create mode 0666; OS-02 `net.accept` ignores its timeout → unbounded blocking / DoS)
- MEDIUM: 3 (OS-03 `canonicalPath`+`isWithin` TOCTOU; OS-04 `openFileNoFollow` guards only the final component; OS-05 `connectTcp`/blocking paths + unbounded `read`/`readText`/`readAll` size)
- LOW: 2 (OS-06 socket fd leak on connect/listen error paths; OS-07 `fs::setCurrentDirectory` mutates process-global CWD across worker threads)
- NTH: 1 (OS-08 `thread.cancel`/`thread.drop` are cooperative-only, detach without join)

---

## OS-01 — HIGH: Files created world-writable (mode 0666) by open/writeText/writeBytes/append

**Location:** `src/target/shared/code/fs_helpers_io.rs:126` (`lower_fs_open_helper`),
`fs_helpers_paths.rs:672` (`lower_fs_write_text_path_helper`), `:1129`
(`lower_fs_write_bytes_path_helper`).

**Issue:** Every non-atomic create path passes mode `438` = octal `0666` as the
third `open` argument:
```
abi::move_immediate("x2", "Integer", "438"),   // 438 == 0o666
```
`open(..., O_CREAT, 0666)` requests world-writable **and** world-readable
permission bits. The final on-disk mode is `0666 & ~umask`, so on a process
with a permissive umask (e.g. `0022` → `0644`, or `0000` → `0666`) freshly
created files are group/other writable or readable. A program that writes a
secret (token, key, config) with `fs::writeText`/`fs::writeBytes`/`fs::openFile("w")`
gets a file readable by any local user, and in the `0000`-umask case writable
by any local user. By contrast `createTempFile` correctly uses mode `384` =
`0600` (`fs_helpers_atomic.rs:111`), showing the intended-secure value exists.

The atomic writers (`lower_fs_atomic_write_helper`) go through `mkstemps`,
whose fd is created `0600` by the C library, so `writeTextAtomic` /
`writeBytesAtomic` are not affected — only the plain writers are.

**Trigger** (real syntax, cf. `tests/func_fs_openFileNoFollow_valid/src/main.mfb`,
`tests/func_fs_createTempFile_valid/src/main.mfb`):
```
IMPORT fs
FUNC main AS Integer
  fs::writeText("/tmp/session-token.txt", "secret-bearer-token")
  RETURN 0
END FUNC
```
On a shared host the token file is created `0644` (default umask) — any local
user can read it. Attacker action: read `/tmp/session-token.txt` as another
user; or, under a `0000` umask, overwrite it with attacker-controlled data.

**Fix:** Change the create-mode literal from `"438"` (0666) to `"384"` (0600)
in `lower_fs_open_helper`, `lower_fs_write_text_path_helper`, and
`lower_fs_write_bytes_path_helper` — matching the temp-file helper. 0600
(owner-only rw) is the correct default for a general file-creation API. This is
a runtime-constant change only; no language surface changes. If a
world-readable file is genuinely wanted the caller can `fs::open` and rely on a
future explicit-mode API, but the safe default must be 0600.

---

## OS-02 — HIGH: `net.accept` ignores its `timeoutMs` argument → unbounded blocking

**Location:** `src/target/shared/code/net/io.rs:16-98` (`lower_net_accept_helper`).

**Issue:** The `accept` spec takes a `timeoutMs` in `x1`
(`net_specs.rs:73-84`, `NET_LISTENER_TIMEOUT_PARAMS`). The helper stores it and
then never uses it:
```
abi::store_u64("x1", abi::stack_pointer(), TIMEOUT_OFFSET),   // saved...
...
// accept(fd, NULL, NULL)
abi::move_register(abi::return_register(), "%v9"),
abi::move_immediate("x1", "Integer", "0"),
abi::move_immediate("x2", "Integer", "0"),
...
platform.emit_libc_call("accept", ...)
```
There is no `poll` on the listener fd before `accept`, and the listener socket
is left blocking (only `connect` toggles `O_NONBLOCK`; `listenTcp` never sets a
receive timeout on the listener). So `accept` blocks indefinitely regardless of
the requested `timeoutMs`. A server that passes a finite `timeoutMs` expecting
to poll for shutdown/other work will instead hang forever waiting for the next
client. `net.poll` and `connectTcp` both correctly implement `poll`-bounded
waits (`poll.rs:47`, `mod.rs:511-537`), so the omission in `accept` is a real
divergence from the contract, not a platform limitation.

**Trigger** (real syntax; `net::listenTcp`/`net::accept` are the documented API):
```
IMPORT net
FUNC main AS Integer
  LET l AS Listener = net::listenTcp("127.0.0.1", 8080, 16)
  ' Expect this to return after ~1s so we can check a shutdown flag:
  LET s AS Socket = net::accept(l, 1000)
  RETURN 0
END FUNC
```
Attacker action: simply never connect. The accept loop blocks forever; a
watchdog/shutdown path that relied on the timeout never runs → denial of
service / unkillable-without-signal server, and no way to bound resource waits.

**Fix:** Before the `accept`, when `timeoutMs > 0`, build a `pollfd { fd,
POLLIN }` and call `poll(&pfd, 1, timeoutMs)` (mirroring `lower_net_poll_helper`
in `poll.rs:40-58`); on `poll == 0` return `ErrTimeout` (as the non-blocking
connect path does at `mod.rs:659-691`), on `< 0` return `ErrNetworkFailed`,
and only fall through to `accept` on readiness. For `timeoutMs <= 0` keep the
current blocking `accept` (documented default). Runtime-helper change only.

---

## OS-03 — MEDIUM: `canonicalPath` + `isWithin` are check-then-use (TOCTOU); not a safe sandbox primitive

**Location:** `src/target/shared/code/fs_helpers_paths.rs:1195` (`lower_fs_canonical_path_helper`),
`:1390` (`lower_fs_is_within_helper`). `isWithin` calls `realpath` on both
inputs (`emit_realpath` at `:1547` and `:1561`) then does a pure in-memory
prefix comparison (`:1571-1608`).

**Issue:** `isWithin(base, child)` resolves `base` and `child` with `realpath`
and returns whether the resolved `child` string is prefixed by the resolved
`base` string. This is the canonical TOCTOU pattern: `realpath` follows every
symlink and returns the resolved name at the instant of the call, but the
helper returns only a boolean — it does not hand back an fd or a resolved path
that the caller then opens. Any subsequent `fs::open`/`fs::readText` re-resolves
the path from scratch. Between the `isWithin` check and the later open, an
attacker who controls any directory component of `child` can swap a component
for a symlink pointing outside `base`, defeating the containment check. The
helper is only as strong as "the filesystem did not change between two
independent syscalls," which is not a security boundary.

Additionally the containment test is a byte-prefix compare with a separator
guard (`:1594-1599`): it correctly rejects `/foo` vs `/foobar` by requiring the
next char to be `/` or NUL, and special-cases root `/` (`:1578-1583`). The
comparison logic itself is sound; the weakness is purely the check-then-use gap
plus the fact that resolution happens once and is discarded.

**Trigger** (real syntax, cf. `tests/func_fs_isWithin_valid/src/main.mfb`):
```
IMPORT fs
FUNC handleUpload AS Integer(rel AS String)
  LET base AS String = "target/uploads"
  LET child AS String = fs::pathJoin([base, rel])
  IF fs::isWithin(base, child) THEN
    LET data AS String = fs::readText(child)   ' re-resolves path here
  END IF
  RETURN 0
END FUNC
```
Attacker action: supply `rel` whose leading directory exists as a real dir when
`isWithin` runs, then (racing) replace that directory with a symlink to `/etc`
before `readText` re-resolves — the guard passed but the read escapes `base`.

**Fix:** This cannot be made race-free without an fd-based API, which would
change the language surface, so the runtime-level mitigation is:
(1) Document `isWithin`/`canonicalPath` as advisory, not a sandbox. (2) Provide
the safe pattern internally: an `openFileNoFollow`-style path that resolves and
opens in a way callers can trust. On Linux, the durable fix is `openat2` with
`RESOLVE_BENEATH`/`RESOLVE_NO_SYMLINKS` (define a new resolve-beneath open
helper that takes base fd + relative path); on macOS there is no `openat2`, so
use `O_NOFOLLOW` per-component via `openat` walking. Given the audit constraint
(runtime-only, no language change), the minimum action is the documentation +
steering callers to `openFileNoFollow`; a full fix requires a new resolve-beneath
runtime helper. Report kept MEDIUM because the current primitive invites unsafe
sandbox use.

---

## OS-04 — MEDIUM: `openFileNoFollow` only protects the final path component

**Location:** `src/target/shared/code/fs_helpers_io.rs:3-201` (`lower_fs_open_helper`
with `no_follow = true`), flag table `fs_helpers_io.rs:1181-1213`
(`open_flag_set`).

**Issue:** `openFileNoFollow` sets the `O_NOFOLLOW` bit in the `open` flags. The
Linux no-follow read flag is `32768` = `0x8000` = `O_NOFOLLOW` (confirmed);
write/rw/append add it too (`33345`/`32834`/`33857`). But `O_NOFOLLOW` only
refuses to follow a symlink **at the final component** of the path — every
intermediate directory in the path is still resolved normally, following any
symlinks among them. So `openFileNoFollow("/a/b/c/file")` still traverses `b`
and `c` as symlinks if they are symlinks; it only guarantees `file` itself is
not a symlink. There is no `openat2`/`RESOLVE_NO_SYMLINKS` usage anywhere
(confirmed by grep). The name and the `ELOOP → ErrAccessDenied` mapping
(`fs_helpers.rs:76-98`) imply stronger protection than delivered.

**Trigger** (real syntax, cf. `tests/func_fs_openFileNoFollow_valid/src/main.mfb`
and the existing `tests/thread-fs-nofollow-symlink-rt`):
```
IMPORT fs
FUNC main AS Integer
  RES f AS File = fs::openFileNoFollow("data/current/secret.txt", "read")
  fs::close(f)
  RETURN 0
END FUNC
```
Attacker action: `secret.txt` is not a symlink (so `O_NOFOLLOW` is satisfied),
but the attacker controls `data/` and points the intermediate `current`
directory symlink at `/root/.ssh`. The open follows `current` and reads
`/root/.ssh/secret.txt`. The final-component guard did nothing.

**Fix:** On Linux, implement the no-follow open with `openat2` and
`open_how { flags, resolve = RESOLVE_NO_SYMLINKS }` (syscall 437) so *every*
component is checked; fall back to component-wise `openat(..., O_NOFOLLOW)`
directory walking where `openat2` is unavailable. On macOS use
`O_NOFOLLOW_ANY` (available on recent macOS) or a component-wise `openat`
walk. At minimum, document that `openFileNoFollow` guards only the last
component so callers do not over-trust it. Runtime-helper change only.

---

## OS-05 — MEDIUM: Unbounded blocking default on `connectTcp` and unbounded allocation in `read`/`readText`/`readAll`/`readAllBytes`

**Location:** connect default: `src/target/shared/code/net/mod.rs:451-457,576-594`
(`lower_net_endpoint_helper`, `timeoutMs <= 0` → plain blocking `connect`).
Read sizing: `net/io.rs:255-262` (`lower_net_read_helper` allocates `maxBytes`),
and file reads `fs_helpers_io.rs:345` (`lower_fs_read_all_helper`),
`:605` (`lower_fs_read_all_bytes_helper`), `fs_helpers_paths.rs:799`
(`lower_fs_read_text_path_helper`) size the allocation from the file's own
reported size (`lseek` end).

**Issue:** Two related availability issues:
1. `net::connectTcp(host, port, timeoutMs)` with `timeoutMs <= 0` takes the
   blocking `connect` branch (`mod.rs:455-457` → `blocking_connect` at `:576`),
   which blocks for the OS default connect timeout (can be minutes on a
   filtered/black-holed host). The spec and the non-blocking branch support a
   bounded connect, but the *default* is unbounded. A caller who passes `0`
   (or omits/deems "no timeout") gets indefinite blocking.
2. `net::read`/`net::readText` allocate a buffer of exactly `maxBytes`
   (`io.rs:259` `move_register(x0, maxBytes)` then `emit_alloc`), so a huge
   `maxBytes` from untrusted input is an arena blow-up in one call. The file
   readers allocate based on the file's `lseek`-reported length with no cap, so
   pointing `fs::readText`/`fs::readAllBytes` at a very large (or `/dev/zero`-like
   growing) file allocates the whole size into the arena.

`net.read` does correctly guard `maxBytes <= 0` → `ErrInvalidArgument`
(`io.rs:255-257`), and UDP `receiveFrom` correctly rejects oversize datagrams
with `ErrMessageTooLarge` (`io.rs:1099-1107`), so the concern is specifically
the unbounded upper end + the connect default.

**Trigger** (real syntax):
```
IMPORT net
FUNC main AS Integer
  ' Attacker-controlled host that silently drops SYNs:
  LET s AS Socket = net::connectTcp("10.255.255.1", 80, 0)   ' blocks for minutes
  RETURN 0
END FUNC
```
Attacker action: a black-holed address (drops packets) makes the `0`-timeout
connect hang the caller. For read: a peer that advertises `maxBytes = 2^40`
worth of a length prefix, or a crafted multi-GB file fed to `fs::readAllBytes`,
forces a giant arena allocation (memory-exhaustion DoS).

**Fix:** (1) For `connectTcp`, treat `timeoutMs <= 0` as a sane finite default
(e.g. a compile-time default constant) rather than the OS-default blocking
connect, or at least document that `0` means "block indefinitely." (2) For the
read helpers, clamp the single-call allocation to a documented maximum and
return `ErrMessageTooLarge`/`ErrOutOfMemory` past it (the datagram path already
demonstrates the pattern at `io.rs:1099-1107`). Both are runtime-helper changes.

---

## OS-06 — LOW: Socket fd leak on connect/listen/bind error paths

**Location:** `src/target/shared/code/net/mod.rs:620-656` (`op_fail`/`socket_fail`
in `lower_net_endpoint_helper`).

**Issue:** The code itself flags this in a comment: after `socket()` succeeds
but a later step fails (e.g. `getsockopt`/`bind`/`listen` errors, or the
non-blocking connect path errors before reaching `op_fail`), some error exits
`close` the fd and some do not. The comment at `:620-622` states: "The socket
fd (if any) leaks on these rare error paths." Each leaked fd consumes a
descriptor; a caller that retries `connectTcp`/`listenTcp` in a loop against a
failing endpoint can exhaust the process fd table.

**Trigger** (real syntax):
```
IMPORT net
FUNC main AS Integer
  FOR i AS Integer = 1 TO 100000
    ' Endpoint that resolves but refuses/errors after socket() succeeds:
    LET s AS Socket = net::connectTcp("127.0.0.1", 1, 100)
  NEXT
  RETURN 0
END FUNC
```
Attacker action: cause repeated post-`socket()` failures (e.g. a port that
resets after SYN, or a resource-limited bind) so the loop leaks one fd per
iteration until `EMFILE`.

**Fix:** Ensure every error label that is reachable after `socket()` returns a
valid fd routes through a single `close(fd)` cleanup before `freeaddrinfo` +
`emit_fail`. Concretely, funnel `op_fail`, the non-blocking connect error exits
(`:534-560`), and `connect_timeout` (already closes) through one
close-then-free epilogue, and audit that `socket_fail` is only entered when no
fd was opened. Runtime-helper change only.

---

## OS-07 — LOW: `fs::setCurrentDirectory` mutates process-global CWD, breaking thread isolation

**Location:** `src/target/shared/code/mod.rs:1236-1241` (routes
`fs.setCurrentDirectory` → `FsPathOperation::Chdir`); syscall names
`macos_aarch64/code.rs:381` (`_chdir`), `linux_aarch64/code.rs:318` (`chdir`),
`linux_x86_64/code.rs:329`.

**Issue:** `setCurrentDirectory` emits `chdir(2)`, which changes the working
directory of the **entire process**, not the calling thread. Threads created by
`thread::start` share the process address space and CWD (they are `pthread`s —
see `runtime_helpers_thread.rs`). So one worker calling
`fs::setCurrentDirectory` silently changes the CWD used by every other worker's
relative-path filesystem operations. Combined with the fact that all the fs
helpers accept relative paths (they `chdir`-relative resolve via the kernel),
this means the thread boundary is not a filesystem-namespace boundary: a
relative `fs::readText("config.txt")` in worker A can resolve to a different
file depending on a concurrent `setCurrentDirectory` in worker B (a race), and
the resolution can be steered.

**Trigger** (real syntax, thread API per `tests/thread-dual-cancel/src/main.mfb`):
```
IMPORT thread
IMPORT fs
IMPORT workers
FUNC main AS Integer
  LET a AS Thread OF String TO String = thread::start(workers::readRel, "config.txt")
  LET b AS Thread OF String TO String = thread::start(workers::chdirTo, "/tmp/evil")
  ' worker b's setCurrentDirectory changes the CWD worker a resolves against
  RETURN 0
END FUNC
```
Attacker action: if any worker processes untrusted input that leads to a
`setCurrentDirectory`, it can redirect the relative-path resolution of every
other concurrent worker (confused-deputy / file-substitution across the thread
boundary).

**Fix:** Document that `setCurrentDirectory` is process-global and not
thread-scoped, and that security-sensitive fs operations must use absolute
paths (or the resolve-beneath open from OS-03/OS-04). A stronger runtime fix
would resolve relative fs paths against a per-thread base directory captured at
`thread::start` rather than the process CWD, but that is a larger change; the
minimum is the documentation + steering to absolute paths. LOW because it
requires a program to actually call `setCurrentDirectory` concurrently.

---

## OS-08 — NTH: `thread.cancel` / `thread.drop` are cooperative-only and detach without join

**Location:** `src/target/shared/code/runtime_helpers_thread.rs:524-580`
(`Cancel`: sets `THREAD_OFFSET_CANCELLED = 1`, closes both queues, broadcasts
conds, `pthread_detach`), `:216-227` and `:568-580` (`pthread_detach` on
drop/waitFor), `thread_is_cancelled_helper` at `:1254-1268`.

**Issue:** `thread::cancel` and `thread::drop` do **not** forcibly terminate the
worker — there is no `pthread_cancel`/`pthread_kill`. They set a cooperative
`CANCELLED` flag (which the worker reads via `thread::isCancelled`) and
`pthread_detach` the OS handle. A worker that never checks `isCancelled` (e.g.
one blocked in a long `fs::readAllBytes`, an unbounded `net::read`, or a tight
compute loop) keeps running to completion after `cancel`/`drop`, holding its
fds/memory. Because the handle is detached (not joined), the parent cannot
observe when — or whether — the worker actually stopped, and resources the
worker owns are released only when it voluntarily exits. This is a deliberate
cooperative-cancellation design (the flag + queue-close is the intended
signal), so it is noted as nice-to-have rather than a defect, but it means
"cancel" is not a hard resource/time bound and should not be relied on to stop
a runaway or malicious worker.

**Trigger** (real syntax, cf. `tests/func_thread_cancel_valid/src/main.mfb`):
```
IMPORT thread
IMPORT thread_workers
FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(thread_workers::echoText, "a", 1, 1)
  thread::cancel(t)   ' worker keeps running if it never polls isCancelled
  RETURN 0
END FUNC
```
Attacker action: a worker whose body ignores `isCancelled` (or is stuck in a
blocking syscall) continues consuming CPU/fds after the parent "cancels" it —
the parent has no forceful stop.

**Fix:** Documentation is the primary action: state clearly that cancellation
is cooperative and that workers must poll `thread::isCancelled` at safe points
(and use bounded `net`/`fs` timeouts) to be stoppable. Optionally, on
`waitFor`/`drop` of a still-running detached worker, add a bounded join wait so
the parent can at least detect a non-terminating worker. Fixing OS-02/OS-05
(bounded network/file waits) is what makes cooperative cancellation actually
effective for blocked workers. No language change.

---

## Checked and OK

- **TLS certificate + hostname verification (OpenSSL backend) is enforced.**
  `tls/openssl.rs` calls `SSL_CTX_set_default_verify_paths` (`:388`),
  `SSL_set_verify(ssl, SSL_VERIFY_PEER, NULL)` (`:445-463`, `SSL_VERIFY_PEER=1`
  from `tls/mod.rs:32`), `SSL_set1_host(ssl, sni)` for hostname checking
  (`:464-483`, failure branches to `tls_fail`), SNI via `SSL_ctrl`
  (`:484-502`), a TLS 1.2 minimum (`SSL_CTRL_SET_MIN_PROTO_VERSION`,
  `:503-509`), and after handshake requires `SSL_get_verify_result == 0`
  (X509_V_OK) with a hard `branch_ne(tls_fail)` (`:530-548`). There is no path
  that skips verification. The SNI/validation name defaults to `host` when
  `serverName` is empty (`:312-339`), so hostname checking is always against a
  real name. (macOS Network.framework backend in `tls/macos.rs` should be
  spot-checked separately, but the OpenSSL path is correct.)
- **`createTempFile` is secure:** `O_RDWR|O_CREAT|O_EXCL` (Linux adds
  `O_CLOEXEC`) — decoded `524482 = 0x800c2`, macOS `2562 = 0xa02`
  (`fs_helpers_atomic.rs:185-194`) — with mode `384 = 0600`
  (`fs_helpers_atomic.rs:111`), and an unpredictable name: `/mfb-` + a UUIDv4
  built from 16 bytes of `emit_random_bytes` with the RFC-4122 version/variant
  nibbles set (`fs_helpers_atomic.rs:196-238`) + `.tmp`. `O_EXCL` defeats the
  predictable-name/symlink attack, and the random UUID defeats guessing.
- **Atomic writers** (`writeTextAtomic`/`writeBytesAtomic`) use `mkstemps`
  (0600 fd), `fsync`, `close`, then `rename` onto the target
  (`fs_helpers_atomic.rs:269-590`) — the correct durable-atomic pattern; no
  world-writable temp.
- **`net.setReadTimeout`/`setWriteTimeout`/`net.poll`** reject negative
  timeouts (`poll.rs:33`, `:128`) and correctly program `SO_RCVTIMEO`/
  `SO_SNDTIMEO` / `poll`.
- **Non-blocking `connectTcp` with `timeoutMs > 0`** is correctly bounded by
  `poll` on `POLLOUT` + `getsockopt(SO_ERROR)` and restores blocking mode
  (`net/mod.rs:454-594`); its timeout path closes the fd and frees the resolver
  (`:659-691`).
- **UDP `receiveFrom`/`receiveTextFrom`** allocate `maxBytes + 1` and reject
  oversize datagrams with `ErrMessageTooLarge` instead of silent truncation
  (`net/io.rs:1074-1107`).
- **Read helpers validate UTF-8** for the text variants
  (`emit_call_validate_utf8` in `fs_helpers_io.rs`, `net/io.rs:313`), returning
  `ErrEncoding` rather than emitting invalid strings.
