# bug-984: on macOS, each `tls::listen` + `tls::close` leaks three arena blocks (2,848 bytes)

Last updated: 2026-10-10
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (resource leak)

Status: Fixed (2026-10-10)
Regression Test: `tests/net/rt_tls_listener_close_frees.rs` (macOS): N × (`tls::listen` + `tls::close`), a listener left to its scope, and three failing `tls::listen` shapes must each leave `alloc_calls == free_calls`, `live_bytes 0`, `double_free_skips 0`

A macOS `tls::listen` allocates six arena blocks, and `tls::close` (plus the scope drop) frees
only three. A server that opens and closes listeners grows by 2,848 bytes each time. The
`tcp::listen` + `tcp::close` control is clean.

**Correct behavior:** after `tls::close` on a listener, every block `tls::listen` allocated
for it has been freed, so `alloc_calls = free_calls` and `live_bytes = 0` at exit, as for
`tcp::`.

## Failing Reproduction

Measured by the coordinator on 2026-10-06 with main at c6dd68acc
(`/tmp/main-dbg-target/release/mfb`), macOS aarch64.

```
openssl req -x509 -newkey rsa:2048 -nodes -keyout key.pem -out cert.pem -days 2 -subj "/CN=127.0.0.1"
mfb init l   # src/main.mfb:
IMPORT tls
IMPORT io

FUNC main AS Integer
  MUT i AS Integer = 0
  WHILE i < 20
    RES s AS tls::Listener = tls::listen("127.0.0.1", 0, "cert.pem", "key.pem")
    tls::close(s)
    i = i + 1
  END WHILE
  io::print("ok")
  RETURN 0
END FUNC

mfb build -q --debug l && ./l/build/l.out
```

Observed (`--debug` report, arena 0):

| iterations | alloc_calls | free_calls | live_bytes |
|---|---|---|---|
| 1 | 6 | 3 | 2848 |
| 2 | 12 | 6 | 5696 |
| 20 | 120 | 60 | 56960 |

Expected: `alloc_calls = free_calls`, `live_bytes 0`.

Control: the same loop with `tcp::listen("127.0.0.1", 0)` + `tcp::close` reports
`alloc_calls 40 free_calls 40 live_bytes 0`.

## Where to look

`lower_tls_listen` (`src/codegen/builtins/tls/gen_macos/server.rs`) declares
`host_scratch_the_listener_owns` and deliberately leaves it out of the helper-scratch
release, on the grounds that the listener owns it. So `tls::close`, or the listener's drop,
must free it, and two more blocks also go unfreed. Use `mfb dbg` with `bp _mfb_arena_alloc bt 2`
and `bp _mfb_arena_free bt 2` on a one-iteration build to name the three blocks.

## Related

- Found by the coordinator while reviewing bug-885 (macOS `tls::listen` host validation).
  bug-885's fix adds a host-resolution buffer that must be freed too; that part is handled
  in bug-885.

## Root Cause

`mfb dbg` on a one-iteration `--debug -O0` build, `bp _mfb_arena_alloc bt 2; list
reg x0` and `bp _mfb_arena_free bt 2; list reg x0; list reg x1`, names every block
`tls::listen` allocates and which are freed (sizes for the repro's 20-byte paths):

| allocated in `tls::listen` at | bytes | what | freed by |
|---|---|---|---|
| `+128` | 20 | cert path C string | `tls::listen` `+4112` (scratch release) |
| `+308` | 1115 | **cert PEM bytes** (`emit_read_whole_file`) | nothing |
| `+424` | 20 | key path C string | `tls::listen` `+4132` (scratch release) |
| `+604` | 1675 | **key PEM bytes** (`emit_read_whole_file`) | nothing |
| `+2240` | 10 | **host C string** (`"127.0.0.1"`) | nothing |
| `+3576` | 112 | the `Listener` record | the binding's drop (`main +1536`) |

The three unfreed blocks are 1120 + 1680 + 16 = 2,816 B after rounding, which is
the per-iteration growth measured for this cert and key (the report's 2,848 B is
the same three blocks with a larger key).

- The two PEM buffers are scratch nobody released. `emit_import_pem_item` hands
  each to `CFDataCreate`, which copies the bytes, so nothing reads the arena block
  afterwards. The key buffer holds the private key.
- The host block has an owner — `tls::listen` parks it in the record at
  `REC_LHOST` for `tls::localAddress(listener)` — but the owner never freed it. The
  code and `.ai/net-tls.md` both said "borrowed, never freed … process-lifetime".

## Resolution

`src/codegen/builtins/tls/gen_macos/server.rs`:

- `emit_read_whole_file` records its buffer in a `HelperScratch` the caller
  declares (`cert_buf_scratch`, `key_buf_scratch`), and both join the guarded
  release at `tls::listen`'s single `done`. Every exit reaches it: success, an
  unreadable file, a short read, a failed import, a failed bind.
- The host block joins that release too, and the success path nulls its scratch
  pointer as it stores the block in the record. So `tls::listen` frees it on every
  exit that built no listener, and never while a listener holds it — the
  use-after-free bug-575 warned of (`tls::localAddress` reading an empty host)
  stays impossible.
- `lower_tls_close_listener_macos` frees the block the record holds, by
  `strlen + 1` (the count both of its allocators use: the marshalled host is
  `byteLength + 1`, and the thread-transfer copy measures the same way), and nulls
  the word. `tls::localAddress(listener)` refuses a closed record before it loads
  the word, and a second close takes the `already` exit before reaching the free.
- The bind-all path used to park the static `_mfb_tls_anyhost` there. It parks an
  arena copy of `"0.0.0.0"` now, so the record's word is always a block its close
  can free, and the close needs no reference to the static (which exists only in a
  module that calls `tls::listen`).

Not fixed (bounded, pre-existing): a listener moved to another thread leaves the
sender's host block allocated. `thread::transfer` copies the string into the
receiver's arena, and the sender's tombstone is freed without it.

### `tests/codegen/codegen_helper_scratch_release.rs` pin (AGENTS.md, four questions)

Two rows of `TLS_HELPERS_NETWORK_FRAMEWORK` change:
`_mfb_rt_tls_tls_listen` (7, 4, 2) → (8, 7, 5) and
`_mfb_rt_tls_tls_closeListener` (0, 0, 0) → (0, 1, 0).

1. **When and why written.** `git log -S"(7, 4, 2)"` on the file: `e0e8ce648`
   (2026-10-06, bug-885) set (7, 4, 2); `f301238de` (2026-09-12, bug-575) wrote the
   row as 7/2/2 with the reasoning that the host copy is not scratch on this
   backend.
2. **What it protects.** Every block a `tls::` helper allocates for marshalling has
   a decided owner: no leak, and no free of a block something still reads (a host
   block released at `done` made `tls::localAddress` read an empty host).
3. **Who else depends on it.** `grep -rn "TLS_HELPERS_NETWORK_FRAMEWORK\|7, 4, 2" .`
   finds this file only. The behaviour the row's comment protects is tested by
   `tests/net/rt_tls_listener_local_address.rs`, which still passes.
4. **Proof the pinned numbers are wrong.** They pin the leak. The debugger table
   above shows three blocks `tls::listen` allocates that no code frees, and the
   `--debug` report shows `live_bytes` growing by 2,816 per `listen` + `close`
   while the `tcp` control is flat. The row's own claim still holds under the new
   numbers: the host block is released at `done` only when no listener holds it
   (the scratch pointer is null on the success path), and its owner frees it in
   `closeListener`.

The new triples are measured, not predicted: the test prints the live counts on a
mismatch. 8 allocations = 7 + the bind-all host copy; 7 frees = 4 + the two PEM
buffers + the host block's failure-path release; 5 guarded releases = 2 + those
three.

### Runtime proof

`mfb build -q --debug`, arena 0, `alloc_calls / free_calls / live_bytes`
(`/tmp/k/probe.sh <mfb> t1 10 20`, the report's loop with `N` from the
environment), base against this branch:

| program | base N=1 | base N=2 | fixed N=10 | fixed N=20 |
|---|---|---|---|---|
| `tls::listen` + `tls::close` | 8 / 5 / 2816 | 15 / 9 / 5632 | 71 / 71 / 0 | 141 / 141 / 0 |
| control: `tcp::listen` + `tcp::close` | 4 / 4 / 0 | 7 / 7 / 0 | 31 / 31 / 0 | 61 / 61 / 0 |

`double_free_skips 0`. The test also covers the bind-all host (`tls::localAddress`
reports `0.0.0.0` while open), a listener nobody closes, and three failing
`tls::listen` shapes (a missing certificate, a file that is not a PEM, a host that
does not resolve), each balanced.

## Golden impact

`.ncode`/`.ncodesum` for `macos-aarch64` move for the fixtures that emit
`_mfb_rt_tls_tls_listen` or `_mfb_rt_tls_tls_closeListener` (the `tls`
byte-identity cover fixture and any program that calls `tls::listen` or drops a
`tls::Listener`). Linux and Windows TLS are untouched. The fixtures the artifact
gate reports are listed in the commit message.
