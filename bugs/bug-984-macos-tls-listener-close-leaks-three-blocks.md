# bug-984: on macOS, each `tls::listen` + `tls::close` leaks three arena blocks (2,848 bytes)

Last updated: 2026-10-06
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (resource leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free-count assertion beside `tests/net/rt_tls_listen_host_validation.rs`: N × (`tls::listen` + `tls::close`) must leave `arena.0.live_bytes 0`

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
