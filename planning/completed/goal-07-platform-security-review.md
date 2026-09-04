# goal-07: MFBASIC platform security review — code-grounded, trust-boundary audit

Last updated: 2026-07-28
Status: NOT STARTED (0 / 8 surfaces audited)

## Objective

Produce a **code-grounded security review** of the MFBASIC platform as it is
implemented today — the language front-end, `.mfp` IR/package decode &
verification, native codegen & runtime helpers, the custom Mach-O / ELF / PE
linker, the fs / net / thread / crypto / tls / term runtime packages, and the
`mfb-repo` package registry service. This is **not** a general bug hunt and
**not** a spec-only read: every finding must be verified against current source
and, where practical, reproduced against a built artifact (`target/debug/mfb`, a
crafted `.mfp`, or the running registry).

This is a **security** review: prioritize attacker-reachable impact —

- **Memory / resource safety** — OOB read/write, use-after-free, double-free,
  unchecked size arithmetic / integer overflow into an allocation, unbounded
  recursion or growth (native codegen + arena / collection / string / SIMD /
  vector runtime).
- **Trust / auth bypass** — missing or forgeable signature/authentication,
  broken challenge/login or session/token handling, authorization gaps in the
  registry, confused-deputy paths, transparency-log or TUF-metadata forgery.
- **Injection** — command / path / format-string / log injection; SSRF from the
  HTTP / net client; ANSI / terminal-escape injection from the `term` / console
  backend.
- **Privilege escalation & sandbox escape** — crossing a boundary the design
  says should hold (author of an untrusted `.mfp` → code that runs at build or
  runtime; registry client → another owner's namespace; one thread → another's
  owned data).
- **Supply chain** — package/dependency substitution, unverified downloads,
  unpinned or spoofable sources, install-time or build-time code execution, a
  dropped-in `.mfp` trusted without signature / hash / IR re-verification.
- **Crypto / verification gaps** — missing signature/hash verification, weak or
  misused primitives (Ed25519 / ECDSA / TLS), predictable secrets, TOCTOU around
  verification, nonce / challenge reuse.
- **Attacker-triggerable DoS** — an untrusted party (remote peer, `.mfp` author,
  registry client) can cheaply exhaust CPU, memory, disk, or handles, or wedge a
  handler indefinitely.
- **Weak hardening** — missing exploit mitigations in emitted binaries
  (PIE/ASLR/NX/RELRO/stack canaries; on Windows: /DYNAMICBASE/NX/CFG), unsafe
  file permissions, secrets in logs/artifacts, information leaks across a
  boundary.

**Out of scope:** pure correctness, polish, or missing features — unless they
create a security-boundary failure. Do not file those here (they belong in a
`create-review` source-review goal, e.g. the `goal-06` lineage).

## Scope

In-scope trees:

- `src/**` — compiler front-end, `.mfp` IR/package decode & verification,
  monomorph, native codegen & runtime helpers, custom linker (Mach-O / ELF /
  PE), CLI, os / term packages.
- `repository/**` — the `mfb-repo` package registry HTTP service (auth,
  transparency log, TUF metadata, blob store, publish/validate, GC).

8 attack surfaces mapped below.

**Editable in this pass:** only `planning/` (audit files) and `bugs/` (bug
documents). This is a **find-and-document** pass — do not fix issues in the
audited code here.

**Out of surface-scope** (with reason):

- `benchmark/`, `examples/`, `tests/`, `tools/` — not attacker-reachable
  production code; a test fixture is in scope only if it masks a real boundary
  gap in `src/**` or `repository/**`.
- Build-time-only host dependencies `image` (PNG-only) and `icns` — used solely
  for macOS `.icns` app-icon generation on the trusted build host; they add
  nothing to emitted programs. Note their pinned versions if a finding touches
  them.
- `third_party/utf8proc`, `bindings/{libsnd,sqlite3}` — vendored third-party
  libraries; audited upstream. In scope only for *how MFBASIC calls into them*
  (untrusted sizes/pointers crossing the FFI boundary), not their internals.
- The compiler's own Rust dependencies (`sha2`, `tinyjson`, `unicode-*`,
  `rusqlite`/bundled SQLite) — supply-chain-pinned via `Cargo.lock`; audit their
  *use*, not their source.

## Threat model — trust boundaries

For each surface, the untrusted party and what they must NOT be able to do.

- **Surface 1 — `.mfp` package decode + verification.** Untrusted party: author
  of a `.mfp` artifact on the dependency path (dropped in locally or fetched
  from a registry). Must not: corrupt memory / crash / execute code in the
  compiler via a malformed or hostile package, nor have unsigned / IR-tampered
  package contents accepted as trusted.
- **Surface 2 — Language front-end.** Untrusted party: author of an arbitrary
  `.mfb` source file the compiler is asked to build. Must not: crash the
  compiler, exhaust memory/CPU (unbounded recursion/growth), or reach codegen
  with invalid IR.
- **Surface 3 — Codegen & runtime memory safety.** Untrusted party: whoever
  controls runtime inputs (sizes, strings, collection contents, cross-thread
  transfers) to a compiled program. Must not: cause OOB access,
  use-after-free/double-free, integer overflow into an allocation, or an
  arena/collection invariant break in emitted code.
- **Surface 4 — fs / net / thread / term runtime helpers.** Untrusted party:
  remote net/http peer; attacker-controlled paths/filenames; hostile terminal
  output content. Must not: read/write outside intended paths, inject via
  CRLF/ANSI/format sinks, SSRF, or wedge/exhaust the process.
- **Surface 5 — Crypto / TLS / verification.** Untrusted party: remote TLS peer;
  author of a signed `.mfp`. Must not: bypass signature/cert verification, force
  a downgrade, exploit a predictable secret/nonce, or a verification TOCTOU.
- **Surface 6 — Custom linker & emitted-binary hardening.** Untrusted party: an
  attacker exploiting an emitted binary at runtime (needs an in-program bug to
  chain against). Must not: find exploit mitigations (PIE/NX/RELRO/canary/CFG)
  silently absent from emitted Mach-O / ELF / PE images.
- **Surface 7 — Package registry HTTP service.** Untrusted party: any remote
  registry client (anonymous or token-holding). Must not: bypass authz across
  owners/namespaces, forge signatures / transparency-log / TUF metadata, take
  over a name, or cheaply exhaust the service.
- **Surface 8 — Supply chain: install / resolve / registry client.** Untrusted
  party: a malicious or MITM'd registry, or a spoofed dependency source. Must
  not: get an unverified / substituted / downgraded package accepted at
  install/build time, or run code at install time.

## Fix constraints (invariants a fix must respect)

Per `AGENTS.md` and the project memory:

- **No language-surface change.** A fix must not alter MFBASIC syntax or
  observable runtime semantics of a correct program. Removed syntax stays fully
  removed (no vestigial reserved words).
- **No wire-format / public-API / registry-contract break.** The `.mfp` binary
  format, the registry HTTP contract (`IndexResponse`, name bindings,
  transparency-log/TUF metadata), and package-index shapes are compatibility
  surfaces — a fix must not silently break them.
- **Never weaken a test or golden to pass.** Follow the `AGENTS.md` "Never edit a
  test/golden to pass" gate: prove the golden wrong (4-point evidence) before
  touching it; findings here are *documentation*, not fixes, so this mostly
  constrains any later fix pass.
- **Production-ready only.** No stubs / placeholders / mocks / fallbacks as a
  "fix"; a fix is a real fix or it is filed as a bug for later.

These are constraints on *proposed fixes* recorded in findings — this pass
applies none of them, but a HIGH/CRITICAL finding's "Best fix" must respect them.

## Prior work — re-verify before re-opening

This platform has been audited twice before; both series are complete and
archived under `planning/completed/`.

- **Audit 1** (`planning/completed/audit-1-*.md`, `goal-01`…`goal-04` lineage) —
  first trust-boundary pass. Four CRITICALs: PKG-01 (`.mfp` signature gate),
  PKG-02 (IR re-verification), MEM-01/02 (string size-overflow).
- **Audit 2** (`planning/completed/audit-2-*.md`, `goal-05-platform-security-review.md`;
  summary `audit-2-summary.md`) — second pass, 8 surfaces, completed 2026-07-14.
  Headline: all four audit-1 CRITICALs and most HIGHs **fixed & re-verified**;
  no CRITICAL and no *new* HIGH found. Still-open HIGHs at that time were
  audit-1 carryovers filed as bugs: **FE-02/FE-03** front-end DoS
  (bug-182/183), **OS-01/OS-02** fs-helper defaults (bug-184/185), **LNK-01**
  non-PIE Linux (bug-186); plus MEDIUMs REPO-12/13 (bug-188), SUP-02/03
  (bug-189), OS-09 CRLF, LNK-08 (bug-187).
  - Per-surface detail: `audit-2-package-decode.md`, `audit-2-frontend.md`,
    `audit-2-codegen-memory.md`, `audit-2-fs-net-thread.md`,
    `audit-2-crypto-tls.md`, `audit-2-linker-hardening.md`,
    `audit-2-repository.md`, `audit-2-supply-chain.md`, plus `audit-unicode.md`.

**New since audit-2** (audit these fresh — no prior coverage): the **Windows PE
target** (`src/target/win_x86_64/**`, `src/os/windows/**`) and its emitted-binary
hardening; the **`linux_gtk` GUI target** (`src/target/linux_gtk/**`); and the
**console/term grid + GUI backends** (plan-13 / plan-70) as an ANSI-escape /
output-injection surface.

Do not re-open a fixed item as a new finding without re-verifying against
*current* code (paths have moved since audit-2 — e.g. `entry_and_arena.rs` →
`arena.rs`, `fs_helpers_*` → `code/fs/*`, `escape.rs` folded into `lexer.rs`,
`shared/validate.rs` → `shared/validate/{body,capabilities,names,mod}.rs`).
Before calling any prior finding fixed or open, check `bugs/`, `bugs/completed/`,
and `bugs/skipped/` for its bug-NN. If a prior finding is still open, reference
its ID rather than duplicating the analysis.

## Severity scale

- **CRITICAL** — attacker-reachable, high-impact, demonstrated (memory
  corruption with control, auth bypass, RCE, supply-chain substitution).
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

- **ID** (surface-prefixed, e.g. `PKG-`, `FE-`, `MEM-`, `OS-`, `CRY-`, `LNK-`,
  `REPO-`, `SUP-`) and **severity** (scale above).
- **Title** and **location** — `path/file.rs:line` (or symbol) cited after a real
  source read.
- **Threat / impact** — who can trigger it and what breaks (confidentiality,
  integrity, availability, trust).
- **Mechanism** — why the code is wrong, not just that it feels risky.
- **Reproduction** — preferred: a minimal input/command against a built binary
  (`target/debug/mfb` on a crafted `.mfb`/`.mfp`, or a request to the running
  `mfb-repo`); if pure decode/protocol/linker, a concrete byte/command repro.
  Record observed vs expected.
- **Best fix** — implementation-level, respecting the fix constraints above.
- **Non-goals** for that fix — what must stay the same.

## Outputs

1. **Audit files**, split by surface (next free audit series number `<N>` =
   **3**):
   - `planning/audit-3-<surface>.md` per surface (e.g.
     `audit-3-package-decode.md`, `audit-3-frontend.md`,
     `audit-3-codegen-memory.md`, `audit-3-fs-net-thread-term.md`,
     `audit-3-crypto-tls.md`, `audit-3-linker-hardening.md`,
     `audit-3-repository.md`, `audit-3-supply-chain.md`).
   - One index: `planning/audit-3-summary.md` with a master finding table (ID,
     severity, title, location, cross-links).
2. **Bug documents** via the **write-bug** skill (or `bugs/bug-NN-<slug>.md` in
   its template) for every **CRITICAL** and **HIGH** finding (and **MEDIUM** when
   the fix is not small). Next free bug number: **394**. Do not implement fixes
   here.

## Method

1. **Map trust boundaries first** (done below; refine as you read).
2. **Fan out by surface** — parallel subagents are fine; each returns findings
   only, with `file:line` citations. Load the `mfbasic` MCP tools (`mfb_man`,
   `mfb_spec`) via `ToolSearch` for spec/behavior questions instead of guessing.
3. **Re-verify every finding yourself** against current source before recording
   it — discard hallucinations and already-fixed items; check the prior-audit
   IDs and their bug-NNs.
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
(`clean`, or the finding ids filed). A file may appear under more than one
surface — the map is by trust boundary, not a partition.

**Surface 1 — Untrusted `.mfp` package decode + signature / IR verification**
_Untrusted party: author of a `.mfp` artifact on the dependency path._

- [ ] `src/binary_repr/reader.rs`
- [ ] `src/binary_repr/sections.rs`
- [ ] `src/binary_repr/util.rs`
- [ ] `src/binary_repr/builder.rs`
- [ ] `src/binary_repr/writer.rs`
- [ ] `src/binary_repr/mod.rs`
- [ ] `src/target/package_mfp/mod.rs`
- [ ] `src/manifest/entry.rs`
- [ ] `src/manifest/package.rs`
- [ ] `src/manifest/mod.rs`
- [ ] `src/manifest/json_edit.rs`
- [ ] `src/target/shared/validate/{mod,body,capabilities,names}.rs`
- [ ] `src/cli/build/**` (signature/hash gate at import/build)
- [ ] `src/cli/resolve.rs`

**Surface 2 — Language front-end (lexer / parser / resolver / syntaxcheck / monomorph / ir)**
_Untrusted party: author of an arbitrary `.mfb` source file._

- [ ] `src/lexer.rs` (includes string-escape decoding)
- [ ] `src/numeric.rs`
- [ ] `src/ast/**` (expr/stmt recursion depth)
- [ ] `src/resolver/**`
- [ ] `src/syntaxcheck/**`
- [ ] `src/monomorph/**` (polymorphic-recursion instantiation)
- [ ] `src/ir/**` (verify / lower)

**Surface 3 — Codegen & runtime memory safety (arena / collections / strings / arithmetic / SIMD / vector)**
_Untrusted party: whoever controls runtime inputs (sizes, strings, transfers)._

- [ ] `src/target/shared/code/arena.rs`
- [ ] `src/target/shared/code/builder_arena_transfer.rs`
- [ ] `src/target/shared/code/builder_strings.rs`
- [ ] `src/target/shared/code/builder_strings_builtins.rs`
- [ ] `src/target/shared/code/builder_strings_package.rs`
- [ ] `src/target/shared/code/builder_collection_{layout,queries,query,compare}.rs`
- [ ] `src/target/shared/code/collection_{buffer,mutate}.rs`
- [ ] `src/target/shared/code/builder_{values,value_semantics,numeric,money,money_math}.rs`
- [ ] `src/target/shared/code/builder_simd_{math,float_math,fixed_math}.rs`, `builder_vector_inline.rs`, `simd_kernel_coeffs.rs`
- [ ] `src/target/shared/code/{runtime_helpers,runtime_helpers_thread,validation}.rs`
- [ ] `src/arch/**`
- [ ] `src/target/{linux_aarch64,linux_x86_64,linux_riscv64,macos_aarch64,win_x86_64,linux_common}/**` (per-target emit)

**Surface 4 — Filesystem / network / thread / terminal runtime helpers**
_Untrusted party: remote net/http peer; attacker-controlled paths/filenames; hostile terminal output content._

- [ ] `src/target/shared/code/fs/{io,paths,atomic,mod}.rs`
- [ ] `src/target/shared/code/builder_fs_paths.rs`
- [ ] `src/target/shared/code/os/{env,introspect,paths,mod}.rs`
- [ ] `src/target/shared/code/{stdin_broadcast,io_stdin}.rs`
- [ ] `src/builtins/{fs,net,http,thread,os,io}.rs`
- [ ] `src/builtins/term.rs` + console/term grid & GUI backends (`src/target/linux_gtk/**`, term draw helpers) — ANSI/escape injection
- [ ] `src/target/shared/runtime/{fs_specs,net_specs,os_specs,thread_specs,io_specs}.rs`

**Surface 5 — Crypto / TLS / verification**
_Untrusted party: remote TLS peer; author of a signed `.mfp`._

- [ ] `src/target/shared/code/crypto_ec.rs` + `src/target/shared/code/crypto_ec/**`
- [ ] `src/target/shared/code/crypto.rs`
- [ ] `src/target/shared/runtime/crypto_specs.rs`
- [ ] `src/builtins/crypto.rs`
- [ ] `src/builtins/tls.rs`
- [ ] Ed25519 `.mfp` signature path (cross-ref Surface 1)
- [ ] `repository/src/crypto.rs` (cross-ref Surface 7)

**Surface 6 — Custom linker & emitted-binary hardening (Mach-O / ELF / PE)**
_Untrusted party: attacker exploiting an emitted binary at runtime._

- [ ] `src/os/linux/link/elf.rs`, `src/os/linux/link/mod.rs`, `src/os/linux/object.rs`
- [ ] `src/os/macos/link/{macho,commands,mod}.rs`, `src/os/macos/object.rs`, `src/os/macos/icon.rs`
- [ ] `src/os/windows/link/{mod,pe,rsrc}.rs`, `src/os/windows/object.rs` (NEW — no prior audit)
- [ ] `src/os/{link_encode,object_plan,note}.rs`

**Surface 7 — Package registry HTTP service (auth / transparency log / TUF metadata / blobs)**
_Untrusted party: any remote registry client (anonymous or token-holding)._

- [ ] `repository/src/server.rs` (all routes: auth/challenge/login, signing, log/*, keys/rotate, machines/*, tokens/*, packages/transfer/*, root/snapshot/timestamp.json, validate, publish, blob, search, rate limits)
- [ ] `repository/src/validation.rs`
- [ ] `repository/src/crypto.rs`
- [ ] `repository/src/abi.rs`
- [ ] `repository/src/store.rs`
- [ ] `repository/src/local.rs`
- [ ] `repository/src/blobstore.rs`
- [ ] `repository/src/package.rs`
- [ ] `repository/src/{log,gc,backfill}.rs`
- [ ] `repository/src/web/mod.rs`
- [ ] `repository/src/main.rs`
- [ ] `repository/docker-entrypoint.sh`, `repository/Dockerfile` (untrusted config only)

**Surface 8 — Supply chain: install / resolve / registry client (compiler side)**
_Untrusted party: malicious or MITM'd registry; spoofed dependency source._

- [ ] `src/cli/pkg.rs`
- [ ] `src/cli/repo.rs`
- [ ] `src/cli/resolve.rs`
- [ ] `src/cli/init.rs`
- [ ] `src/manifest/{url,libraries}.rs`
- [ ] `repository/src/client.rs`
- [ ] cross-ref Surface 1 (`.mfp` verification) and Surface 5 (signature crypto)
