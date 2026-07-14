# Audit 2 — Platform security review: summary & index

Last updated: 2026-07-14
Status: COMPLETE (8 / 8 surfaces audited)

Second code-grounded, trust-boundary security review of the MFBASIC platform
(compiler front-end, `.mfp` decode + verification, native codegen & runtime,
custom Mach-O/ELF linker, fs/net/thread/crypto/tls helpers, and the `mfb-repo`
registry). This audit is the successor to `old-plans/audit-1-*`; every prior
CRITICAL/HIGH was re-verified against current source, and each finding here cites
`file:line` from a real read (with reproductions run against `target/debug/mfb`,
crafted inputs, or the registry where practical). **Find-and-document pass — no
fixes applied.** Next free bug number after this pass: **190**.

## Headline

The platform is **materially more hardened than at audit-1.** All four audit-1
CRITICAL findings (PKG-01 signature gate, PKG-02 IR re-verification, MEM-01/02
string size-overflow) and most HIGH findings are **fixed and re-verified**. The
crypto/TLS surface (new since audit-1) is fail-closed with no bypass. The registry
authorization model (rotate/transfer/tokens/orgs/signing) has **no demonstrable
cross-owner bypass, forgery, or namespace takeover.** Supply-chain install is
**not blind** — two independent §3.5 signature checks (install + build), SHA-256
blobs, no install-time code execution.

**No CRITICAL and no *new* HIGH were found.** The five HIGH items are all
**still-open audit-1 findings** re-verified against current code (two frontend DoS
crashes, two fs-helper defaults, one Linux linker-hardening gap). The remaining
new work is concentrated in availability (DoS) and defense-in-depth hardening.

## Files in this audit

| File | Surface | Findings |
|---|---|---|
| [audit-2-package-decode.md](audit-2-package-decode.md) | `.mfp` decode + signature/IR verification | PKG-01..07 fixed; PKG-08 |
| [audit-2-frontend.md](audit-2-frontend.md) | lexer/parser/resolver/monomorph/ir | FE-01/04/05 fixed; FE-02, FE-03 |
| [audit-2-codegen-memory.md](audit-2-codegen-memory.md) | arena/collections/strings/arith/SIMD | MEM-01..08 fixed; MEM-09, MEM-10 |
| [audit-2-fs-net-thread.md](audit-2-fs-net-thread.md) | fs / net / thread helpers | OS-01..05 open; OS-09/10/11 |
| [audit-2-crypto-tls.md](audit-2-crypto-tls.md) | crypto / TLS / verification (new) | CRY-01/02/03 |
| [audit-2-linker-hardening.md](audit-2-linker-hardening.md) | Mach-O/ELF writers & hardening | LNK-01/02/03/05/07 open; LNK-08..11 |
| [audit-2-repository.md](audit-2-repository.md) | registry HTTP service | REPO-04/09 open; REPO-12..19 |
| [audit-2-supply-chain.md](audit-2-supply-chain.md) | install / resolve / registry client | SUP-01/02/03; SUP-04 (positive) |

## Master finding table (this audit's open items)

### CRITICAL (0)
_None. All four audit-1 CRITICAL findings are fixed and re-verified._

### HIGH (5) — all still-open audit-1 items, re-verified

| ID | Title | Location | Repro | Bug doc |
|---|---|---|---|---|
| FE-02 | Monomorph polymorphic recursion → unbounded instantiation → SIGABRT | `src/monomorph/lower.rs:475,639` | ✓ built binary | bug-182 |
| FE-03 | Statement-block recursion has no parser depth limit → SIGABRT before ir::verify | `src/ast/stmt.rs:710` | ✓ built binary (N≥2000) | bug-183 |
| OS-01 | File-creating fs builtins open with mode 0o666 → world-readable/writable secrets | `fs_helpers_io.rs:700`, `fs_helpers_atomic.rs:920,1442` | ✓ code | bug-184 |
| OS-02 | `net.accept` ignores its `timeoutMs` → indefinite block / DoS | `net/io.rs:47,55-61` | ✓ code | bug-185 |
| LNK-01 | Linux binaries non-PIE (`ET_EXEC` @ `0x400000`) → main-image ASLR defeated | `elf.rs:30`, `mod.rs:7` | ✓ readelf | bug-186 |

### MEDIUM (12)

| ID | Title | Location | Bug doc |
|---|---|---|---|
| OS-09 | HTTP request header CRLF injection (request splitting) | `http_package.mfb:131` | — (small; in audit file) |
| SUP-02 | First-contact TOFU on the registry server key (bootstrap MITM) | `local.rs:242`, `client.rs:26` | bug-189 |
| SUP-03 | Unauthenticated `/index` version list → signature-preserving downgrade | `client.rs:889-914`, `pkg.rs:593` | bug-189 |
| LNK-08 | No read-only data segment: program constants writable at runtime | `elf.rs:54-67`, `macho.rs:152-158` | bug-187 |
| REPO-13 | No rate-limit/quota on `/validate`+`/publish` → CPU/disk exhaustion | `server.rs:1712,1720` | bug-188 |
| REPO-12 | Global (non-per-client) rate-limit buckets on register/login → lockout DoS | `server.rs:681,1531` | — (small; paired w/ bug-188) |
| OS-03 | `canonicalPath`+`isWithin` check-then-open TOCTOU (no `openat2`) | `fs_helpers_paths.rs:1410` | — (open audit-1) |
| OS-04 | `openFileNoFollow` guards only the final component | `fs_helpers_io.rs:2191` | — (open audit-1) |
| OS-05 | Unbounded default connect + unbounded read allocation | `net/mod.rs:493`, `net/io.rs:314` | — (open audit-1) |
| LNK-02 | No `PT_GNU_STACK` header (exec-stack policy left to loader) | `elf.rs` phdr list | — (fold into bug-186) |
| LNK-03 (Linux) | No RELRO; GOT writable despite `DF_BIND_NOW` | `elf.rs:453,632` | — (fold into bug-186) |
| REPO-09 | Single global `Mutex<Connection>` + permanent poison on panic | `store.rs:13` | — (open audit-1; ~LOW-MED) |

### LOW (19)

PKG-08 (Scalar const escapes literal-range verifier, `ir/verify/mod.rs:1626`) ·
MEM-09 (bin-park double-free guard parity gap, `entry_and_arena.rs:1655,1673`) ·
OS-07 (process-global `chdir`) · OS-08 (cooperative-only cancel) · OS-10 (HTTP
SSRF, no redirect vector) · OS-11 (HTTP client no timeouts) · CRY-01 (macOS TLS no
min-version floor) · CRY-02 (Ed25519 software-core malleable S) · LNK-05 (no
BTI/PAC note) · LNK-07 (unchecked reloc slice writes, build-time) · LNK-09 (no
stack canaries) · LNK-10 (macOS no hardened runtime; `__DATA_CONST` maxprot RW) ·
LNK-11 (object-encoder branch truncation) · REPO-04 (JWT no aud/iss) · REPO-14
(SQL LIKE wildcard cross-package log match) · REPO-15 (link/fetch auth-key gated
only by `lookup`) · REPO-16 (uncached tree/index recompute on anonymous reads) ·
REPO-17 (no charset/length on package/version) · SUP-01 (plaintext-http default).

### NTH (3)
CRY-03 (`constantTimeEqual` length leak) · REPO-18 (TUF 1-of-1 threshold; online
keys share serving DB) · REPO-19 (transparency log has no witness/gossip →
split-view).

### Not demonstrated / positive (2)
MEM-10 (copy/transfer/SIMD size multiplies diverge from checked-helper convention
— unreachable) · SUP-04 (install is *not* blind — full §3.5 verification confirmed).

Tallies: **CRITICAL 0 · HIGH 5 · MEDIUM 12 · LOW 19 · NTH 3** (+ 2
not-demonstrated). All 5 HIGH are re-verified still-open audit-1 items; 0 new HIGH.

## Bug documents filed (CRITICAL/HIGH + non-small MEDIUM)

| Bug | Finding(s) | Severity | Effort |
|---|---|---|---|
| bug-182 | FE-02 monomorph recursion | HIGH | medium |
| bug-183 | FE-03 statement-block recursion | HIGH | small |
| bug-184 | OS-01 world-writable file mode | HIGH | small |
| bug-185 | OS-02 net.accept ignores timeout | HIGH | medium |
| bug-186 | LNK-01 Linux non-PIE | HIGH | x-large |
| bug-187 | LNK-08 writable program constants | MEDIUM | large |
| bug-188 | REPO-13 (+ REPO-12) validate/publish quota + rate limit | MEDIUM | large |
| bug-189 | SUP-02/03 (+ SUP-01) bootstrap TOFU + downgrade + plaintext | MEDIUM | large |

Small MEDIUM/LOW/NTH items are documented in the per-surface files with a best-fix
sketch but no bug doc (per the goal: bug docs for CRITICAL/HIGH and non-small
MEDIUM only).

## Cross-cutting themes

1. **`ir::verify` has the right caps but sits last.** FE-02/FE-03 both crash an
   earlier uncapped pass (monomorph / parser) before `ir::verify`'s `MAX_DEPTH`
   backstop can reject the input. The fixes belong upstream.
2. **Availability is the dominant residual class.** FE-02/03, OS-02/05/11,
   REPO-12/13/16, and the cooperative-cancel amplifier OS-08 are all DoS. No new
   memory-corruption or auth-bypass primitive was demonstrated.
3. **Linux executable hardening is the remaining pre-distribution gap** (LNK-01
   non-PIE, LNK-02 GNU_STACK, LNK-03 RELRO, LNK-08 writable rodata) — best done as
   one PIE/segment rework (bug-186 + bug-187). macOS is materially ahead
   (`MH_PIE`, `SG_READ_ONLY`).
4. **Registry trust is strong at the core, opt-in at the edges.** Signature/authz
   are sound; the gaps are making anti-rollback/bootstrap-pinning *mandatory*
   (SUP-02/03) and adding per-owner throttling/quota (REPO-12/13).

## Recommended remediation order

1. **Compiler DoS (cheap, clear):** bug-183 (parser statement-depth cap), bug-182
   (monomorph instantiation cap).
2. **fs/net defaults:** bug-184 (0o600 file mode), bug-185 (accept timeout).
3. **Registry availability:** bug-188 (per-owner throttle+quota) + REPO-12
   (per-client register/login buckets).
4. **Supply-chain trust:** bug-189 (pin bootstrap + mandatory snapshot/downgrade
   protection + https).
5. **Before shipping user-distributed Linux binaries:** bug-186 (PIE, folding in
   GNU_STACK/RELRO) + bug-187 (read-only constants).
6. **Then the LOW/NTH hardening** per the individual surface files (PKG-08 Scalar
   verifier, MEM-09 bin-park guard, OS-03/04 TOCTOU, CRY-01 macOS TLS floor,
   REPO-14 LIKE escaping, etc.).
