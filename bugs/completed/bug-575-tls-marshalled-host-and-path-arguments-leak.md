# bug-575: the twelve `tls::` C-string marshalling sites leak their argument

Last updated: 2026-09-12
Effort: medium (mechanical per site; two sites needed the scratch threaded out of a shared sub-emitter, and two of the three backends cannot be exercised on this host)
Severity: MEDIUM (unbounded leak on `tls::connect`/`tls::listen`, proportional to the host name / certificate path length)
Class: Memory / correctness

Status: **FIXED**
Regression Test:
`tests/runtime/rt_scope_drop_leaks.rs::a_tls_connect_host_name_is_not_leaked_in_proportion_to_its_length`
— the RSS half, measured on the Network.framework and OpenSSL backends;
`tests/codegen/codegen_helper_scratch_release.rs` — `TLS_HELPERS_OPENSSL`,
`TLS_HELPERS_NETWORK_FRAMEWORK` and `TLS_HELPERS_SCHANNEL`, one cross-built
`(arena_alloc, arena_free, guarded release)` table per backend, plus
`every_tls_marshalling_site_is_handed_a_scratch`;
`tests/net/rt_tls_listener_local_address.rs::tls_listen_binds_every_interface_when_the_host_is_empty`
— the positive pin on the one path that reaches `ret` without marshalling.

Split out of bug-574, which fixed the same defect in `fs`, `os`, `net`, `tcp` and
`udp`.

## The defect

`tls` has its own copy of the marshaller —
`emit_cstring` in `src/codegen/builtins/tls/gen_shared.rs`, distinct from the
`os/socket/shared.rs` one bug-574 converted. It allocated `len + 1` arena bytes,
copied the `String` in NUL-terminated, stored the pointer at `sp + out_off`, and
returned. Nothing freed it, on any path — the same shape, the same consequence:
every `tls::connect`/`tls::listen` leaked its host name and its certificate/key
paths, in proportion to their length.

Twelve call sites, all three backends:

| file | sites |
| --- | --- |
| `builtins/tls/gen_openssl.rs` | 6 (143, 405, 417, 1172, 1296, 1306) |
| `builtins/tls/gen_macos/client.rs` | 2 (122, 241) |
| `builtins/tls/gen_macos/server.rs` | 2 (82, 844) |
| `builtins/tls/gen_schannel_impl.rs` | 1 (51) |
| `builtins/tls/gen_schannel_server.rs` | 1 (333) |

**The enumeration and the attribution both held.** All twelve sites were where the
report said, and the leak is exactly the block `emit_cstring` allocates: measured
on Linux/OpenSSL at 20 000 → 40 000 iterations of a failing `tls::connect`, peak
RSS grew **2 048 B per call for a 1 613-character host and 8 192 B per call for a
6 413-character one** — the arena bin `len + 1` rounds up to, once per call, with
nothing else in the reading.

## Measurements

`mfb` built from `197d84d02` (before) and from this change (after). Peak RSS of a
loop of failing `tls::connect` calls — the host is marshalled BEFORE it is
resolved, which is why a failing call reaches the leak at all. The name is far
past the 253-byte DNS limit, so no resolver queries the network for it.

| backend | host chars | 20 000 iterations, before | after |
| --- | --- | --- | --- |
| Network.framework (macOS aarch64) | 1 613 | 173.8 MB | 44.8 MB |
| Network.framework (macOS aarch64) | 6 413 | 333.3 MB | 43.5 MB |
| OpenSSL (Linux x86_64 musl, box 2227) | 1 613 | 40.4 MB | 8.4 MB |
| OpenSSL (Linux x86_64 musl, box 2227) | 6 413 | 160.3 MB | 5.5 MB |
| OpenSSL (Linux x86_64 glibc, box 2228) | 6 413 | 162.2 MB | — |

The regression test asserts the length SENSITIVITY, not flatness: the difference
between the two host lengths at one iteration count. Before it is +159.5 MB
(macOS) / +119.9 MB (Linux); after it is negative on both — the longer run is the
cheaper one, because a released block is one the arena can hand back.

Schannel could not be exercised: the three backends are mutually exclusive per
platform and no Windows runtime was available. It is pinned by cross-built
codegen only (`TLS_HELPERS_SCHANNEL`), the same instrument that pins the OpenSSL
and Network.framework emitters.

## The fix

The shape bug-574 landed: `HelperScratch::declare` at the top of each enclosing
helper body — ahead of every branch that can reach `done`, not at the allocation —
`emit_cstring` taking the declared scratch and recording the block's pointer and
the exact byte count `_mfb_arena_alloc` was given, and
`emit_helper_scratch_release` immediately before the single `ret`.

Per helper:

| helper | scratch blocks |
| --- | --- |
| OpenSSL `connect`/`connectAddr` | host, SNI (the `snihost`/`sni` arms are exclusive halves of one choice writing one slot, so they share it) |
| OpenSSL `listen` | host, cert path, key path |
| Network.framework `connect`/`connectAddr` | host, SNI |
| Network.framework `listen` | cert path, key path (**not** the host — see below) |
| Schannel `connect`/`connectAddr` | host |
| Schannel `listen` | host |

Two sites sit in shared sub-emitters with no `ret` of their own, so the scratch is
declared by the caller and threaded in rather than the emitter being duplicated:

* `gen_macos/server.rs::emit_read_whole_file` takes a `path_scratch`.
  `lower_tls_listen_macos` calls it TWICE, for the certificate and for the key,
  through the same `PATHCSTR` frame slot — so it declares two, or the first block
  would be overwritten and lost.
* `gen_schannel_impl.rs::socket_connect` takes a `host_scratch`;
  `lower_tls_connect` declares it and releases it at its own `done`.

### One of the twelve is NOT scratch

`lower_tls_listen_macos` marshals three C-strings and only two of them are the
helper's. The host copy is **owned by the `Listener`**: `tls::listen` parks it in
the record at `REC_LHOST`, and `tls::localAddress(listener)` reads it back for the
listener's whole life, because `nw_listener_get_port` answers the port and
Network.framework answers no address at all (bug-465). The record store has said
so since that bug — "Borrowed, never freed" — and this change nearly did it
anyway: the first version released all three, and
`tls_local_address_reports_the_port_a_listener_bound_to` failed with

```
could not parse a port out of "bound  62586\n"
```

— the port intact, the host an empty string, because the block had gone back to
the arena before anyone asked for it. **The bind-all form does not catch this**:
that path parks the static `_mfb_tls_anyhost` rodata pointer and reads back
`"0.0.0.0"` either way, so the new positive pin was green while the old one was
red. The row in `TLS_HELPERS_NETWORK_FRAMEWORK` is 7/**2**/**2** and says why.

### The null-init dominates

Three of the six converted helpers have a path that reaches `ret` without
marshalling anything, and every one of them is the shape bug-574 got wrong once in
`net::listen`:

* **`tls::listen` with an empty host** (all three backends) branches to
  `null_host` and jumps straight past its `emit_cstring`, storing a plain 0
  (OpenSSL, Schannel) or the address of the static `"0.0.0.0"` rodata string
  (Network.framework) in the `HOSTCSTR` slot. A release keyed on that frame slot,
  or one whose pointer vreg were initialised at the allocation, would free rodata.
* **`tls::connect` with a negative `timeoutMs`** raises `ErrInvalidArgument`
  before any marshalling.
* The `alloc_fail`/`ErrOutOfMemory` tail is itself such a path.

`HelperScratch::declare` nulls both vregs at the top of the body, and
`emit_helper_scratch_release` is a runtime pointer guard, so all three free
nothing. The empty-host form is pinned behaviourally, not just structurally:
`tls_listen_binds_every_interface_when_the_host_is_empty` binds `""`, reads the
port back, completes a real handshake against it with `openssl s_client`, requires
the payload, and requires the server to exit 0 rather than on a signal.

## Residue, measured rather than assumed

**`tls::listen` on macOS still leaks its host copy, once per listener.** Nothing
frees `REC_LHOST` — not `tls::close(listener)` — and that is the pre-existing,
deliberate decision the record store documents. It is bounded by the number of
listeners a program creates rather than by calls in a loop, it is not the defect
this bug is about, and freeing it correctly needs the block's SIZE carried in the
record beside the pointer. Left alone and recorded here.

A **failing** `tls::connect` still grows ~260 B per call on Linux and ~1.9 KB per
call on macOS with the leak fixed, and that residue is length-INDEPENDENT
(identical at 1 613 and 6 413 host characters). It is not bug-575 and not `tls`:
`tcp::connect` in the identical trapped-failure shape — whose marshalling bug-574
already fixed — grows by the same ~260 B per call on the same box. It is the
trapped-error path, in the same family as bug-574's own "an UNBOUND
runtime-helper `String` result has no owner" note. Not touched here; it is why the
RSS case asserts length-insensitivity rather than flatness, and the test says so.

## Gates

Recorded in the commit.
