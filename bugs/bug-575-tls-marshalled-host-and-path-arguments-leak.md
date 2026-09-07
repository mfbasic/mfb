# bug-575: the twelve `tls::` C-string marshalling sites leak their argument

Last updated: 2026-09-07
Effort: medium (mechanical per site; two sites need the scratch threaded out of a shared sub-emitter, and two of the three backends cannot be exercised on this host)
Severity: MEDIUM (unbounded leak on `tls::connect`/`tls::listen`, proportional to the host name / certificate path length)
Class: Memory / correctness

Status: Open
Regression Test: pinned as a NEGATIVE (still-leaks) case —
`tests/codegen/codegen_helper_scratch_release.rs::the_tls_marshalling_sites_are_the_known_residue`
asserts the site COUNT is still 12, so converting one forces this document to be
closed and the helper's row moved into that file's per-package tables.

Split out of bug-574, which fixed the same defect in `fs`, `os`, `net`, `tcp` and
`udp`.

## The defect

`tls` has its own copy of the marshaller —
`emit_cstring` in `src/codegen/builtins/tls/gen_shared.rs`, distinct from the
`os/socket/shared.rs` one bug-574 converted. It allocates `len + 1` arena bytes,
copies the `String` in NUL-terminated, stores the pointer at `sp + out_off`, and
returns. Nothing frees it, on any path — the same shape, the same consequence:
every `tls::connect`/`tls::listen` leaks its host name and its certificate/key
paths, in proportion to their length.

Twelve call sites, all three backends:

| file | sites |
| --- | --- |
| `builtins/tls/gen_openssl.rs` | 6 (143, 405, 417, 1172, 1296, 1306) |
| `builtins/tls/gen_macos/client.rs` | 2 (122, 241) |
| `builtins/tls/gen_macos/server.rs` | 2 (82, 844) |
| `builtins/tls/gen_schannel_impl.rs` | 1 (51) |
| `builtins/tls/gen_schannel_server.rs` | 1 (333) |

## Why it was not done with bug-574

Two reasons, both about the safety of the release rather than its difficulty.

1. **Two sites are in shared sub-emitters.** `gen_macos/server.rs:82` is inside
   `emit_read_whole_file` and `gen_schannel_impl.rs:51` is inside
   `socket_connect`; neither owns the `done` the release has to sit at, so the
   `HelperScratch` has to be declared by the caller and threaded in. bug-574's
   `HelperScratch::declare` / `emit_helper_scratch_release` pair already supports
   that shape (`os::setEnv` declares two), but it is a signature change through
   each sub-emitter rather than a local edit.
2. **The null-init must dominate.** A scratch release at `done` is only safe if
   the pointer vreg is nulled ahead of EVERY branch that can reach `done` —
   bug-574 hit exactly this in `net::listen`, whose empty-host bind-all path jumps
   straight past its `emit_cstring`. Establishing that per site needs the body
   read, and the OpenSSL and Schannel bodies cannot be exercised on the macOS host
   this was measured on: the three backends are mutually exclusive per platform,
   so a mistake in two of the three would only surface in CI, as a wild free.

## What a fix must produce

The same shape bug-574 landed: `HelperScratch::declare` at the top of each
enclosing helper body, `emit_cstring` taking the declared scratch, and
`emit_helper_scratch_release` immediately before the `ret`. Then move each helper
into the per-package table in `tests/codegen/codegen_helper_scratch_release.rs`
with its `(arena_alloc, arena_free, guarded release)` triple, drop the residue
count, and add a `tls::connect` RSS case with a long host name to
`tests/runtime/rt_scope_drop_leaks.rs`.

Measure as peak RSS at N and 2N with a LONG argument: bug-574's short-path row was
~65 B per call, easy to lose in chunk-growth noise, while its 415-byte row was
~1 819 B and unmistakable.
