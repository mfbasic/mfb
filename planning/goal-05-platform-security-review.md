# goal-05: MFBASIC platform security review — code-grounded, trust-boundary audit

Last updated: 2026-07-13
Status: NOT STARTED (0 / 8 surfaces audited)

## Objective

Produce a **code-grounded security review** of the MFBASIC platform as it is
implemented today — the language front-end, IR/package decode, native codegen &
runtime, the custom Mach-O/ELF linker, the runtime helper packages (fs / net /
thread / crypto / tls / audio / os / term), and the package registry service
(`mfb-repo`). This is **not** a general bug hunt and **not** a spec-only read:
every finding must be verified against current source and, where practical,
reproduced against a built artifact (`target/debug/mfb`, a crafted `.mfp`, or the
running registry).

This is a **security** review: prioritize attacker-reachable impact —

- **Memory / resource safety** — OOB read/write, use-after-free, double-free,
  unchecked size arithmetic / integer overflow into an allocation, unbounded
  recursion or growth (native codegen + arena/collection/string/SIMD runtime).
- **Trust / auth bypass** — missing or forgeable signature/authentication,
  broken challenge/login or session/token handling, authorization gaps in the
  registry, confused-deputy paths, transparency-log or TUF-metadata forgery.
- **Injection** — command/path/format-string/log injection; SSRF from the HTTP /
  net client; ANSI/terminal-escape injection from the `term` backend.
- **Privilege escalation & sandbox escape** — crossing a boundary the design
  says should hold (author of an untrusted `.mfp` → code that runs at build or
  runtime; registry client → another owner's namespace; one thread → another's
  owned data).
- **Supply chain** — package/dependency substitution, unverified downloads,
  unpinned or spoofable sources, install-time or build-time code execution, a
  dropped-in `.mfp` trusted without signature/hash/IR re-verification.
- **Crypto / verification gaps** — missing signature/hash verification, weak or
  misused primitives (Ed25519 / ECDSA / TLS), predictable secrets, TOCTOU around
  verification, nonce/challenge reuse.
- **Attacker-triggerable DoS** — an untrusted party (remote peer, `.mfp` author,
  registry client) can cheaply exhaust CPU, memory, disk, or handles, or wedge a
  handler indefinitely.
- **Weak hardening** — missing exploit mitigations in emitted binaries
  (PIE/ASLR/NX/RELRO/stack canaries), unsafe file permissions, secrets in
  logs/artifacts, information leaks across a boundary.

**Out of scope:** pure correctness, polish, or missing features — unless they
create a security-boundary failure. Do not file those here (they belong in the
`goal-04` source review).

## Scope

In-scope trees:

- `src/**` — compiler front-end, IR/package decode, monomorph, native codegen &
  runtime helpers, custom linker, CLI, os package.
- `repository/**` — the `mfb-repo` package registry HTTP service (auth,
  transparency log, TUF metadata, blob store, publish/validate).

8 attack surfaces mapped below.

**Editable in this pass:** only `planning/` (audit files) and `bugs/` (bug
documents). This is a **find-and-document** pass — do not fix issues in the
audited code here.

**Out of surface-scope** (with reason):

- `src/docs/**`, `src/testing*`, `tests/**`, `benchmark/**` — docs, test
  harness, and fixtures; audit only if a fixture masks a real boundary gap.
- Third-party crates (`ed25519-dalek`, `security-framework`, OpenSSL, `axum`,
  `sqlx`/store backend, `p256`/ECDSA libs) — audited upstream; audit *our usage*
  (key handling, verification calls, error paths), not their internals. Note
  versions from `Cargo.lock` when a usage finding depends on them.
- Build tooling (`scripts/**`, `tools/**`, `Dockerfile`, CI) — not
  attacker-reachable at runtime; audit only the registry `docker-entrypoint.sh`
  if it handles untrusted config.
- Generated build artifacts (`target/**`, `repository/target/**`) — outputs, not
  source.

## Threat model — trust boundaries

For each surface, the untrusted party and what they must NOT be able to do.

- **`.mfp` package decode + verification** — untrusted party: the author of a
  `.mfp` artifact dropped into the package cache / dependency path. Must not:
  cause the compiler to trust unsigned/tampered bytes, inject type-confused or
  linearity-violating IR that codegens to unsafe native code, or crash/DoS the
  build via unbounded recursion, OOM, or size-arithmetic overflow during decode.
- **Language front-end** — untrusted party: author of an arbitrary `.mfb` source
  file compiled by the user. Must not: crash the compiler with unbounded
  parse/resolve/monomorph recursion (stack-overflow SIGABRT), or drive codegen
  into an unchecked state.
- **Codegen & runtime memory safety** — untrusted party: whoever controls
  program inputs at runtime (attacker-supplied strings, collection sizes, thread
  transfers, SIMD/vector lengths). Must not: reach an OOB read/write, UAF,
  double-free, or size-overflow-driven under-allocation in emitted native code or
  runtime helpers.
- **Filesystem / network / thread runtime helpers** — untrusted party: a remote
  peer feeding the net/http client, or attacker-controlled paths/filenames. Must
  not: escape an intended directory (path traversal / TOCTOU), create
  world-writable secrets, wedge a handler indefinitely (missing timeout), trigger
  SSRF, or corrupt cross-thread ownership transfer.
- **Crypto / TLS / verification** — untrusted party: a remote TLS peer, or the
  author of a signed `.mfp`. Must not: bypass certificate/signature verification,
  exploit a weak/misused primitive or predictable secret, or leak key material.
- **Custom linker & executable hardening** — untrusted party: an attacker
  exploiting an emitted binary at runtime. Must not: benefit from disabled
  mitigations (non-PIE / no-NX / no-RELRO / no-canary) that the platform should
  provide by default.
- **Package registry service** — untrusted party: any remote registry client
  (anonymous or holding a scoped token). Must not: publish into or transfer
  another owner's namespace, forge auth challenges/logins/tokens, forge
  transparency-log or TUF (root/snapshot/timestamp) metadata, poison the blob
  store, or DoS/disk-fill the service.
- **Supply chain: install / resolve / registry client** — untrusted party: a
  malicious or MITM'd registry, or a spoofed dependency source. Must not: get an
  unverified or substituted package accepted, execute code at install/resolve
  time, or downgrade/pin-bypass a dependency.

## Fix constraints (invariants a fix must respect)

- **Do not change the MFBASIC language surface** — syntax, observable runtime
  semantics, and built-in signatures stay as documented (`mfb spec` / `mfb man`).
  Security fixes are internal (decode hardening, size-check insertion, codegen
  guards).
- **Do not change the `.mfp` / MFPC wire format or the scalar/package ABI**
  unless a finding is *about* the format; if a format change is truly required,
  call it out explicitly as such.
- **Registry fixes are ordinary service-code changes** — the registry is a
  service, not part of the language, so its fixes may change service behavior,
  storage, and HTTP responses as needed.
- Follow `AGENTS.md`: production-ready only, commit on the current branch, never
  tree-wide `git restore`, never blanket-revert a real fix.

## Prior work — re-verify before re-opening

A substantial prior audit exists — **treat it as the baseline, not a to-do
list.** Most of its CRITICAL/HIGH findings were fixed (see the memory index:
`audit-1-package-decode-impl` PKG-01..07 landed; `goal-03` closed bugs 153–180).
Re-verify every prior finding against *current* code before recording anything.

- `planning/old-plans/audit-1-summary.md` — master index of the prior review
  (MFBASIC compiler, linker, runtime, registry). Sub-files:
  - `audit-1-package-decode.md` — `.mfp` decode + signature/IR verification
    (PKG-01..07).
  - `audit-1-codegen-memory.md` — arena / collections / strings / arithmetic
    memory safety (MEM-01..08).
  - `audit-1-frontend.md` — lexer/parser/resolver/typecheck/monomorph recursion
    (FE-01..05).
  - `audit-1-fs-net-thread.md` — filesystem / network / thread helpers
    (OS-01..08).
  - `audit-1-linker-hardening.md` — Mach-O/ELF writers & executable hardening
    (LNK-01..07).
  - `audit-1-repository.md` — registry HTTP service + plan-10 gaps (REPO-01..11).
  - `audit-unicode.md` — Unicode table/runtime surface.
- `bugs/bug-96-audit-collector-missing-tls-http-crypto.md` — a prior gap noting
  the audit collector did not cover tls/http/crypto; confirm current coverage.
- An older spec-only `security-review-1.md` predates the source tree; superseded
  by audit-1. Ignore unless it names an untested boundary.

**New since audit-1 — no prior coverage, audit fresh:** crypto ECDSA
(`crypto_ec.rs`, `builtins/crypto.rs`), TLS (`builtins/tls.rs`), HTTP/net client
(`builtins/http.rs`, `builtins/net.rs`), audio device I/O (`builtins/audio.rs`),
the `term` TUI/ANSI backend (`builtins/term.rs`, `term_grid.rs`), the scalar wire
format & package-ABI renumber (plan-41), RVV/RISC-V vector codegen, the iOS
target, and the registry's transparency-log / TUF-metadata / machine-link / token
routes.

Do not re-open a fixed item as a new finding without re-verifying against current
code. If a prior finding is still open, reference its ID (e.g. `LNK-01`) rather
than duplicating the analysis.

## Severity scale

- **CRITICAL** — attacker-reachable, high-impact, demonstrated (memory
  corruption with control, auth bypass, RCE, supply-chain substitution, signature
  bypass).
- **HIGH** — serious impact, reachable, strong evidence even if not fully
  weaponized.
- **MEDIUM** — real boundary weakness with limited impact or preconditions.
- **LOW** — defense-in-depth / latent; code path exists but no plausible trigger
  constructed.
- **NTH** — nice-to-have hardening.

Label only what you can support. **"Not demonstrated"** is an allowed, honest
label when a path exists but you could not exercise it — do not promote those to
CRITICAL/HIGH without evidence.

## Finding requirements

Each finding must include:

- **ID** (surface prefix + number, e.g. `PKG-`, `FE-`, `MEM-`, `OS-`, `CRY-`,
  `LNK-`, `REPO-`, `SUP-`) and **severity**.
- **Title** and **location** — `path/file.rs:line` (or symbol) cited after a real
  source read.
- **Threat / impact** — who can trigger it and what breaks (confidentiality,
  integrity, availability, trust).
- **Mechanism** — why the code is wrong, not just that it feels risky.
- **Reproduction** — preferred: a minimal input/command against a built binary (a
  crafted `.mfb`, a byte-crafted `.mfp`, a `curl` against `mfb-repo`); if pure
  decode/protocol/linker, a concrete byte/command repro. Record observed vs
  expected.
- **Best fix** — implementation-level, respecting the fix constraints above.
- **Non-goals** for that fix — what must stay the same.

## Outputs

1. **Audit files**, split by surface — audit series `<N>` = **2** (audit-1 is the
   prior review):
   - `planning/audit-2-<surface>.md` per surface (e.g.
     `audit-2-package-decode.md`, `audit-2-frontend.md`,
     `audit-2-codegen-memory.md`, `audit-2-fs-net-thread.md`,
     `audit-2-crypto-tls.md`, `audit-2-linker-hardening.md`,
     `audit-2-repository.md`, `audit-2-supply-chain.md`).
   - One index: `planning/audit-2-summary.md` with a master finding table (ID,
     severity, title, location, cross-links).
2. **Bug documents** via the **write-bug skill** (falls back to
   `bugs/bug-NN-<slug>.md`) for every **CRITICAL** and **HIGH** finding (and
   **MEDIUM** when the fix is not small). Next free bug number: **182**. Do not
   implement fixes here.

## Method

1. **Map trust boundaries first** (done above; refine as you read).
2. **Fan out by surface** — parallel subagents are fine; each returns findings
   only, with `file:line` citations, and does not fix anything.
3. **Re-verify every finding yourself** against current source before recording
   it — discard hallucinations and already-fixed audit-1 items.
4. **Write the audit files and summary; file bug docs** for CRITICAL/HIGH (and
   qualifying MEDIUM).
5. **Do not implement fixes in this pass.**

## Findings ledger

Update as findings are filed.

| ID | Surface | Title | Severity | Repro | Bug doc |
|----|---------|-------|----------|-------|---------|
| _(none yet)_ | | | | | |

Tallies: CRITICAL 0 · HIGH 0 · MEDIUM 0 · LOW 0 · NTH 0.

## Attack-surface map & progress

Audited by surface. Mark `- [x]` with a verdict when a surface is fully covered
(`clean`, or the finding ids filed). A file may appear under more than one surface
— the map is by trust boundary, not a partition.

**Surface 1 — Untrusted `.mfp` package decode + signature / IR verification**
_Untrusted party: author of a `.mfp` artifact on the dependency path._

- [ ] `src/binary_repr/reader.rs`
- [ ] `src/binary_repr/sections.rs`
- [ ] `src/binary_repr/util.rs`
- [ ] `src/binary_repr/builder.rs`
- [ ] `src/binary_repr/mod.rs`
- [ ] `src/target/package_mfp/mod.rs`
- [ ] `src/manifest/entry.rs`
- [ ] `src/manifest/package.rs`
- [ ] `src/manifest/mod.rs`
- [ ] `src/target/shared/validate.rs`
- [ ] `src/cli/build.rs` (signature/hash gate at import/build)
- [ ] `src/cli/resolve.rs`

**Surface 2 — Language front-end (lexer / parser / resolver / syntaxcheck / monomorph)**
_Untrusted party: author of an arbitrary `.mfb` source file._

- [ ] `src/lexer.rs`
- [ ] `src/escape.rs`
- [ ] `src/numeric.rs`
- [ ] `src/ast/**` (expr/stmt recursion depth)
- [ ] `src/resolver/**`
- [ ] `src/syntaxcheck/**`
- [ ] `src/monomorph/**` (polymorphic-recursion instantiation)
- [ ] `src/ir/**` (verify / lower)

**Surface 3 — Codegen & runtime memory safety (arena / collections / strings / arithmetic / SIMD / vector)**
_Untrusted party: whoever controls runtime inputs (sizes, strings, transfers)._

- [ ] `src/target/shared/code/entry_and_arena.rs`
- [ ] `src/target/shared/code/builder_arena_transfer.rs`
- [ ] `src/target/shared/code/builder_strings.rs`
- [ ] `src/target/shared/code/builder_strings_builtins.rs`
- [ ] `src/target/shared/code/builder_collection_layout.rs`
- [ ] `src/target/shared/code/builder_collection_queries.rs`
- [ ] `src/target/shared/code/builder_values.rs`
- [ ] `src/target/shared/code/builder_numeric.rs`
- [ ] `src/target/shared/code/builder_money_math.rs`
- [ ] `src/target/shared/code/builder_simd_math.rs`
- [ ] `src/target/shared/code/builder_simd_float_math.rs`
- [ ] `src/target/shared/code/builder_vector_inline.rs`
- [ ] `src/target/shared/code/runtime_helpers.rs`
- [ ] `src/target/shared/code/validation.rs`
- [ ] `src/arch/**`, `src/target/{linux_aarch64,linux_x86_64,linux_riscv64,macos_aarch64}/**` (per-target emit)

**Surface 4 — Filesystem / network / thread runtime helpers**
_Untrusted party: remote net/http peer; attacker-controlled paths/filenames._

- [ ] `src/target/shared/code/fs_helpers_io.rs`
- [ ] `src/target/shared/code/fs_helpers_paths.rs`
- [ ] `src/target/shared/code/fs_helpers_atomic.rs`
- [ ] `src/target/shared/code/builder_fs_paths.rs`
- [ ] `src/target/shared/code/os.rs`
- [ ] `src/target/shared/code/stdin_broadcast.rs`
- [ ] `src/builtins/{fs,net,http,thread,os,io}.rs`
- [ ] `src/target/shared/runtime/{fs_specs,net_specs,os_specs,thread_specs,io_specs}.rs`

**Surface 5 — Crypto / TLS / verification**
_Untrusted party: remote TLS peer; author of a signed `.mfp`._

- [ ] `src/target/shared/code/crypto_ec.rs`
- [ ] `src/target/shared/runtime/crypto_specs.rs`
- [ ] `src/builtins/crypto.rs`
- [ ] `src/builtins/tls.rs`
- [ ] Ed25519 `.mfp` signature path (cross-ref Surface 1)
- [ ] `repository/src/crypto.rs` (cross-ref Surface 7)

**Surface 6 — Custom linker & emitted-binary hardening (Mach-O / ELF)**
_Untrusted party: attacker exploiting an emitted binary at runtime._

- [ ] `src/os/linux/link/elf.rs`
- [ ] `src/os/linux/link/mod.rs`
- [ ] `src/os/linux/object.rs`
- [ ] `src/os/macos/link/macho.rs`
- [ ] `src/os/macos/link/commands.rs`
- [ ] `src/os/macos/link/mod.rs`
- [ ] `src/os/macos/object.rs`
- [ ] `src/os/macos/icon.rs`

**Surface 7 — Package registry HTTP service (auth / transparency log / TUF metadata / blobs)**
_Untrusted party: any remote registry client (anonymous or token-holding)._

- [ ] `repository/src/server.rs` (all routes: auth/challenge/login, signing, log/*, keys/rotate, machines/*, tokens/*, packages/transfer/*, root/snapshot/timestamp.json, validate, publish, blob)
- [ ] `repository/src/validation.rs`
- [ ] `repository/src/crypto.rs`
- [ ] `repository/src/abi.rs`
- [ ] `repository/src/store.rs`
- [ ] `repository/src/local.rs`
- [ ] `repository/src/blobstore.rs`
- [ ] `repository/src/package.rs`
- [ ] `repository/src/log.rs`
- [ ] `repository/docker-entrypoint.sh` (untrusted config only)

**Surface 8 — Supply chain: install / resolve / registry client (compiler side)**
_Untrusted party: malicious or MITM'd registry; spoofed dependency source._

- [ ] `src/cli/pkg.rs`
- [ ] `src/cli/repo.rs`
- [ ] `src/cli/resolve.rs`
- [ ] `src/cli/init.rs`
- [ ] `repository/src/client.rs`
- [ ] cross-ref Surface 1 (`.mfp` verification) and Surface 5 (signature crypto)
