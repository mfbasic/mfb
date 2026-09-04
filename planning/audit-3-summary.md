# Audit 3 — Platform security review: summary & index

Last updated: 2026-09-03
Status: IN PROGRESS (1 / 9 surfaces written up)

Third code-grounded, trust-boundary security review of the MFBASIC platform,
executed from `planning/goal-08-platform-security-review.md`. Successor to
`planning/completed/audit-1-*` and `planning/completed/audit-2-*`. Every finding
cites `file:line` from a real read, re-verified by the lead against current
source; reproductions were run against `target/debug/mfb`, crafted inputs, or a
compiled program where practical. **Find-and-document pass — no fixes applied.**

Next free bug number at the start of this pass: **489** (measured:
`ls bugs bugs/completed bugs/skipped | grep -oE 'bug-[0-9]+' | sort -n | tail`
→ 488; `git log --all --grep='bug-4[89][0-9]'` → no bug-489+ on any branch).

## Prior-audit carryover, re-measured at the start of this pass

audit-2 closed with eight items filed as bugs. Their state today
(`find bugs -name 'bug-18[2-9]-*.md'` + `grep -m1 -i '^Status' <each>`):

| bug | audit-2 ID | location today | Status line |
|---|---|---|---|
| bug-182 | FE-02 monomorph polymorphic recursion | `bugs/completed/` | Fixed |
| bug-183 | FE-03 stmt-block parser recursion | `bugs/completed/` | Fixed |
| bug-184 | OS-01 world-writable file mode | `bugs/completed/` | Fixed |
| bug-185 | OS-02 `net.accept` ignores timeout | `bugs/completed/` | Fixed |
| bug-186 | LNK-01 non-PIE Linux binaries | `bugs/completed/` | Fixed (dynamic path; RELRO deferred to bug-187) |
| bug-187 | LNK-08 writable program constants | `bugs/completed/` | Fixed on Linux (3 arches) + macOS aarch64 |
| bug-188 | REPO-12/13 registry publish/validate quota | `bugs/completed/` | Fixed |
| bug-189 | SUP-02/03 bootstrap TOFU + version downgrade | `bugs/skipped/` | **Partially Fixed — SUP-03 downgrade defense remaining** |

So exactly one audit-2 carryover is still open — **bug-189 / SUP-03**, the
registry version-list downgrade — and it is parked in `bugs/skipped/`.

## Files in this audit

| File | Surface | Findings |
|---|---|---|
| [audit-3-package-decode.md](audit-3-package-decode.md) | 1 — `.mfp` decode + signature/IR verification | PKG-01 (LOW); audit-1 PKG-01..07 + audit-2 PKG-08 re-verified fixed |
| [audit-3-frontend.md](audit-3-frontend.md) | 2 — lexer / parser / resolver / monomorph / IR / optimizer | _pending_ |
| [audit-3-codegen-memory.md](audit-3-codegen-memory.md) | 3 — arena / collections / strings / engine / backends / threads / canvas | _pending_ |
| [audit-3-os-runtime.md](audit-3-os-runtime.md) | 4 — fs / net / http / process / thread / term / app | _pending_ |
| [audit-3-decoders.md](audit-3-decoders.md) | 5 — encoding / json / csv / regex / PNG / font / MML | _pending_ |
| [audit-3-crypto-tls.md](audit-3-crypto-tls.md) | 6 — crypto / TLS / verification | _pending_ |
| [audit-3-linker-hardening.md](audit-3-linker-hardening.md) | 7 — Mach-O / ELF / PE / AppImage hardening | _pending_ |
| [audit-3-repository.md](audit-3-repository.md) | 8 — registry HTTP service | _pending_ |
| [audit-3-supply-chain.md](audit-3-supply-chain.md) | 9 — install / resolve / registry client | _pending_ |

## Master finding table

_Filled in as surfaces complete; the authoritative running ledger is the
Findings ledger in `goal-08-platform-security-review.md`._

### CRITICAL

_none yet_

### HIGH

_none yet_

### MEDIUM

_none yet_

### LOW

| ID | Surface | Title | Location | Spike |
|---|---|---|---|---|
| PKG-01 | 1 | Signature gate and codegen-feeding decode are separate, unsynchronised reads of the same path (TOCTOU) | `src/cli/build/packages.rs:159` vs `src/binary_repr/mod.rs:869` | n/a — not MFB-expressible |

### NTH

_none yet_

## What is new in this audit's scope

Surfaces with **no prior security coverage** (they postdate audit-2), called out
because a clean verdict on them is a weaker signal than a clean verdict on a
twice-audited surface:

- the **Windows PE target** (`src/target/win_x86_64/**`, `src/os/windows/**`)
  and its emitted-binary hardening;
- the **`linux_gtk` GUI target** and the **app** package's window-system input;
- the **canvas GPU path** (Metal / Vulkan) and its three-thread scene ring;
- **canvas PNG / inflate / font** decoding — untrusted image and font files;
- the **audio MML** parser and synth;
- **http gzip / chunked / multipart / cookie** handling;
- the **encoding / csv / regex / process** builtin packages.
