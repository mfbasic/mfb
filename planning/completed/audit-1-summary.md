# Audit 1 — Security Review Summary & Index

Code-grounded security review of the MFBASIC language, compiler, custom linker, runtime,
and package registry service, plus the `plan-10-*` registry design docs. Every finding
below was verified by reading the actual source (and, for the frontend, reproduced against
a freshly built `target/debug/mfb`); each cites `file:line`. Fixes are implementation-level
and do **not** change the MFBASIC language surface (the registry is a service, so its fixes
are ordinary service-code changes).

This differs from the earlier `security-review-1.md`, which was a spec-only review done
without the source tree. This audit is grounded in the code as coded.

## Files in this audit

| File | Surface | Findings |
|---|---|---|
| [audit-1-package-decode.md](audit-1-package-decode.md) | `.mfp` untrusted binary decode + signature/IR verification | PKG-01..07 |
| [audit-1-codegen-memory.md](audit-1-codegen-memory.md) | Codegen & runtime memory safety (arena, collections, strings, arithmetic) | MEM-01..08 |
| [audit-1-frontend.md](audit-1-frontend.md) | Lexer / parser / resolver / typecheck / monomorph | FE-01..05 |
| [audit-1-fs-net-thread.md](audit-1-fs-net-thread.md) | Filesystem / network / thread runtime helpers | OS-01..08 |
| [audit-1-linker-hardening.md](audit-1-linker-hardening.md) | Custom Mach-O/ELF writers & executable hardening | LNK-01..07 |
| [audit-1-repository.md](audit-1-repository.md) | Package registry HTTP service + plan-10 design gaps | REPO-01..11 |

## Master finding table

### CRITICAL (4)

| ID | Title | Location |
|---|---|---|
| PKG-01 | Compiler never verifies the `.mfp` Ed25519 signature or recomputes the content hash at import/build — any dropped-in package is trusted | `src/target/package_mfp/`, `src/cli/build.rs` |
| PKG-02 | Decoded package IR is never re-typechecked (no type/resource-linearity/Result checks) → malicious `.mfp` injects type-confused IR that codegens to memory-unsafe native code | `src/binary_repr/reader.rs`, `src/target/shared/validate.rs` |
| MEM-01 | `strings.repeat` computes `len * times` as an unchecked 64-bit multiply → wrap → under-alloc → copy loop writes true size → heap OOB write | `builder_strings_builtins.rs:1713` |
| MEM-02 | `strings.padLeft/padRight` size math (`pad_count * padLen`) unchecked → same under-alloc → OOB write | `builder_strings_builtins.rs:1907` |

### HIGH (11)

| ID | Title | Location |
|---|---|---|
| PKG-03 | Unbounded recursion decoding nested IR ops/values → stack-overflow DoS | `src/binary_repr/reader.rs` |
| PKG-04 | `decode_type_name` has no cycle guard → self-referential type payload recurses forever | `src/binary_repr/reader.rs` |
| MEM-03 | Arena double-free: free-list insert lacks idempotency guard + scrub-before-insert poisons coalesce words → overlapping free list / UAF | `entry_and_arena.rs:1076,1180` |
| MEM-04 | Thread-transfer collection copy sizes dest from unchecked source header math → wrap → undersized buffer → OOB | `builder_arena_transfer.rs:387` |
| FE-01 | Parser expression recursion has no depth guard → stack overflow (SIGABRT) | `src/ast/expr.rs` |
| FE-02 | Monomorph polymorphic recursion → unbounded type instantiation → SIGABRT | `src/monomorph/lower.rs:1288` |
| FE-03 | Statement-block recursion has no depth limit → stack overflow (SIGABRT) | `src/ast/stmt.rs:692` |
| OS-01 | `open`/`writeText`/`writeBytes`/append create files world-writable `0o666` (secrets land world-readable) | `fs_helpers_io.rs:126`, `fs_helpers_paths.rs:672,1129` |
| OS-02 | `net.accept` ignores its `timeoutMs` → indefinite block / DoS | `net/io.rs:16` |
| LNK-01 | Linux binaries are non-PIE (`ET_EXEC` at fixed `0x400000`) → main-image ASLR fully defeated | `os/linux/link/elf.rs:14,73,143` |
| REPO-01 | `publish_package` writes the blob to disk before the ownership DB tx commits → orphan blobs, disk-fill DoS, reuse of unverified bytes | `repository/src/server.rs:300` |

### MEDIUM (15)

| ID | Title | Location |
|---|---|---|
| PKG-05 | `Vec::with_capacity(count)` sized from untrusted counts (~13 sites) → OOM before validation | `src/binary_repr/reader.rs` |
| PKG-06 | Duplicate MFPC section IDs silently accepted (last-wins) → decode/verify desync | `src/binary_repr/sections.rs` |
| MEM-05 | `toBytes`/`graphemes`/`split` size multiplies unchecked (OOM-first) | `builder_strings*.rs` |
| MEM-06 | Arena coalescing trusts caller-passed free size → can't detect oversized/overlapping free extent | `entry_and_arena.rs` |
| FE-04 | Unbounded `Vec::with_capacity(count)` in package decoder (frontend view of PKG-05) | `src/binary_repr/reader.rs:67` |
| OS-03 | `canonicalPath`+`isWithin` check-then-open TOCTOU (no `openat2`/`RESOLVE_BENEATH`) | `fs_helpers_paths.rs` |
| OS-04 | `openFileNoFollow` guards only the final component (intermediate dir symlinks followed) | `fs_helpers_paths.rs` |
| OS-05 | Unbounded default connect + unbounded read allocation | `net/`, `fs_helpers_io.rs` |
| LNK-02 | No `PT_GNU_STACK` header emitted (exec-stack policy left to loader; RWX risk on static x86) | `os/linux/link/elf.rs` |
| LNK-03 | No RELRO on Linux; macOS `__DATA_CONST` missing `SG_READ_ONLY` → GOT writable at runtime | `os/linux/link/`, `os/macos/link/` |
| LNK-04 | AArch64 static ELF maps text+data in one R+X `PT_LOAD` → constant data executable | `os/linux/link/elf.rs:32` |
| REPO-02 | No request-size cap; `Json` + base64 `artifact` decode → memory-exhaustion DoS (`/validate` cheapest) | `repository/src/server.rs` |
| REPO-03 | No rate limiting on register/challenge/login; unknown-owner reply is an enumeration oracle; per-call Ed25519 cost | `repository/src/server.rs` |
| REPO-05 | `/validate` requires only *a* session (not the owner's) + leaks per-field key-mismatch → cross-owner oracle | `repository/src/server.rs` |
| REPO-09 | Single global `Mutex<Connection>` serializes all DB access + permanent poison on panic → DoS amplifier | `repository/src/store.rs` |

### LOW / NTH (16)

| ID | Title | Location |
|---|---|---|
| PKG-07 | Unchecked `pos + n` / `offset + N` in `IrReader` (not exploitable at 64-bit bounds; defense-in-depth) | `src/binary_repr/reader.rs` |
| MEM-07 | `arena_alloc` size-normalization `+15` has no overflow guard (root amplifier for MEM-01/02/04) | `entry_and_arena.rs` |
| MEM-08 | `math.abs` hardcodes `x17` (safe under LinearScan today; hardening) | `builder_math.rs` |
| FE-05 | Bare untyped Float literal overflow to `inf` not caught (silent wrong value) | `src/typecheck/checking.rs:230` |
| OS-06 | Socket-fd leak on connect/listen error paths (self-noted in code) | `net/` |
| OS-07 | `fs::setCurrentDirectory` = process-global `chdir` breaks thread CWD isolation | `fs_helpers.rs` |
| OS-08 | `thread.cancel`/`drop` are cooperative-only (no `pthread_cancel`/join) | `runtime_helpers_thread.rs` |
| LNK-05 | No AArch64 BTI/PAC `GNU_PROPERTY` note / no landing pads | `os/linux/link/`, `arch/aarch64` |
| LNK-06 | `branch_imm26`/`page21`/x86 `rel32` truncate out-of-range deltas silently (correctness as images grow) | `arch/*/encode/` |
| LNK-07 | Relocation application uses unchecked slice writes (build-time panic, not memory corruption) | `os/*/link/` |
| REPO-04 | JWT lacks `aud`/`iss` binding + no secret-rotation path (alg pinned + secret persisted = OK otherwise) | `repository/src/crypto.rs` |
| REPO-06 | `LocalPaths::*_path` interpolates raw `owner` (safe only via caller-side validation; latent traversal) | `repository/src/local.rs` |
| REPO-07 | Blob filename from `content_hash` never asserted `^[0-9a-f]{64}$` before FS use (brittle) | `repository/src/server.rs` |
| REPO-08 | `/blob` on-read hash re-verification not implemented (design gap; must survive to plan-10-A A2) | `plan-10-A-keys-install.md` |
| REPO-10 | `logEntry` is a fake `publish:{uuid}`; no transparency log/Merkle/proofs exist (plan-10-C C1) | `repository/src/server.rs` |
| REPO-11 | Three-role key model aliased to one `auth` key; `registration_message` has no role discriminator (cross-role proof replay when split) | `repository/src/server.rs:262` |

## Cross-cutting themes

1. **Unchecked size arithmetic is the dominant memory-safety class.** MEM-01/02/04/05 and
   MEM-07 all stem from computing an allocation size (`a * b`, `a + b`) in 64-bit without an
   overflow check, then addressing the true (larger) size. The fix pattern is uniform: a
   `umulh`/carry check before `ARENA_ALLOC_SYMBOL`, routed to the existing
   `emit_invalid_argument_return` / `emit_allocation_error_return`. The
   `emit_checked_integer_multiply` machinery already exists — these paths just don't call it.

2. **The `.mfp` import path has no trust boundary.** PKG-01 (no signature check) + PKG-02
   (no IR re-verification) together mean a hostile package is a direct path to memory-unsafe
   generated code in the victim's binary. Real crypto exists only in the `repository` crate
   at publish time, not at consume time.

3. **DoS via missing bounds/limits/depth-guards is pervasive** across decoder (PKG-03/04/05),
   frontend (FE-01/02/03), runtime (OS-05), and registry (REPO-02/03/09).

4. **Executable hardening is incomplete on Linux** (LNK-01/02/03/04): non-PIE, no RELRO, no
   GNU_STACK, executable constant data — each amplifies any of the memory bugs above.

## Recommended remediation order

**Fix first (CRITICAL, cheap, high blast-radius):**
1. MEM-01, MEM-02 — add overflow-checked size math to `strings.repeat`/`padLeft`/`padRight`
   (and MEM-07 `arena_alloc` normalization as the shared amplifier). One-check-per-site.
2. MEM-04 — overflow-check the thread-transfer copy size.
3. MEM-03 — arena free-list idempotency guard + stop scrubbing before coalesce read.

**Before importing any non-first-party package:**
4. PKG-01 — verify Ed25519 signature + recompute content hash at import/build (reuse
   `mfb_repository::crypto`).
5. PKG-02 — run the full source-level verifier (type/resource/Result checks) over decoded
   package IR; treat serialized flags as claims to recompute.
6. PKG-03/04/05 — depth limits + count caps + overflow-checked offsets in the decoder.

**Before shipping user-distributed binaries:**
7. LNK-01 (PIE on Linux), LNK-03 (RELRO + `SG_READ_ONLY`), LNK-04 (split R+X/R+W),
   LNK-02 (`PT_GNU_STACK`).

**Before exposing the registry off localhost:**
8. REPO-01 (commit-then-write blobs), REPO-02 (body-size cap), REPO-03 (rate limiting),
   REPO-09 (drop the global mutex / WAL).

**Compiler DoS hardening:**
9. FE-01/03 (shared parser depth counter), FE-02 (instantiation-depth cap in monomorph).

**Then the MEDIUM/LOW hardening** (fs perms OS-01, accept timeout OS-02, TOCTOU OS-03/04,
JWT `aud`/`iss` REPO-04, transparency log REPO-10, etc.) per the individual files.
