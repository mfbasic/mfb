# Audit 3 — Platform security review: summary & index

Last updated: 2026-09-03
Status: COMPLETE (9 / 9 surfaces)

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
| [audit-3-package-decode.md](audit-3-package-decode.md) | 1 — `.mfp` decode + signature/IR verification | PKG-01 (LOW), PKG-10 (LOW); audit-1 PKG-01..07 + audit-2 PKG-08 re-verified fixed |
| [audit-3-frontend.md](audit-3-frontend.md) | 2 — lexer / parser / resolver / monomorph / IR / optimizer | FE-01/02/03 HIGH; FE-04/05/06 + FE-50/51 MEDIUM; IR verifier holds |
| [audit-3-codegen-memory.md](audit-3-codegen-memory.md) | 3 — arena / collections / strings / engine / backends / threads / canvas | MEM-11/12/40/70 HIGH; MEM-41/71/72/13 MEDIUM; GPU path + backend sweep clean |
| [audit-3-os-runtime.md](audit-3-os-runtime.md) | 4 — fs / net / http / process / thread / term / app | **OS-50 CRITICAL**; OS-01/02/51/52/53/54/55 HIGH; ~12 MEDIUM |
| [audit-3-decoders.md](audit-3-decoders.md) | 5 — encoding / json / csv / regex / PNG / font / MML | DEC-01/02/03/50/51/53/54/55 HIGH (all DoS); no memory corruption |
| [audit-3-crypto-tls.md](audit-3-crypto-tls.md) | 6 — crypto / TLS / verification | CRY-50 HIGH; CRY-01/51/52/53 MEDIUM; TLS trust core sound |
| [audit-3-linker-hardening.md](audit-3-linker-hardening.md) | 7 — Mach-O / ELF / PE / AppImage hardening | LNK-12/13 HIGH; LNK-14/15/16 MEDIUM; linker not a parser (clean) |
| [audit-3-repository.md](audit-3-repository.md) | 8 — registry HTTP service | REPO-01/02/03 HIGH (authz bypass); 7 MEDIUM; crypto/log/TUF fail-closed |
| [audit-3-supply-chain.md](audit-3-supply-chain.md) | 9 — install / resolve / registry client | SUP-01/02/03/04 MEDIUM; transport/install core sound |

## Master finding table

The authoritative running ledger (with spikes, repro status, and bug numbers) is
the Findings ledger in `goal-08-platform-security-review.md`. Headline below.

**CRITICAL 1 · HIGH 28** enumerated; MEDIUM ≈ 40 · LOW ≈ 22 · NTH ≈ 10 across the
nine surface files. New bug docs: **bug-489 … bug-512** (24), plus bug-189
augmented. Every CRITICAL and HIGH has a bug doc and (where MFB-expressible) a
spike under `spikes/audit-3/`.

### CRITICAL (1)

| ID | Surface | Title | Location | Bug |
|---|---|---|---|---|
| OS-50 | 4 | `tcp/tls/udp` write of a `String`-returning call selects the byte-list lowering → remote peer-controlled OOB read (lead-reproduced: 22 B request → 1024 B of process memory) | `src/codegen/engine/value/builder_values.rs:2419` | bug-497 |

### HIGH (28)

| ID | Surface | Title | Bug |
|---|---|---|---|
| REPO-01 | 8 | Scoped publish token self-escalates to permanent unscoped auth key (live) | bug-492 |
| REPO-02 | 8 | `/machines/revoke` accepts an auth-key challenge → account lockout (live) | bug-493 |
| REPO-03 | 8 | `/release-state`+`/signing` authorize on ident prefix, not owner (live) | bug-494 |
| MEM-11 | 3 | Bounds-check elision on stale length → OOB heap read (live, all -O) | bug-495 |
| MEM-12 | 3 | `g=<op>(g,f())` reads freed block (UAF) via `&`/append (live) | bug-496 |
| MEM-70 | 3 | `thread::send` allocates on peer arena unlocked → heap corruption (live) | bug-498 |
| MEM-40 | 3 | Win64 entry seed scratch aliases stdin buffer slot → arbitrary ptr R/W | bug-512 |
| OS-01/04 | 4 | Spawned children inherit fds/sockets (no CLOEXEC / bInheritHandles) | bug-499 |
| OS-02 | 4 | env-replace clear loop infinite-loops (no `=` entry) | bug-500 |
| OS-53/54/55 | 4 | HTTP CRLF injection + request-smuggling toolbox | bug-506 |
| OS-51/52/56 | 4 | HTTP server DoS (chunk-abort / slowloris / quadratic head) | bug-507 |
| FE-01 | 2 | operator-chain compiler stack overflow (live) | bug-501 |
| FE-02 | 2 | `mfb fmt` quadratic blowup + non-atomic source overwrite | bug-502 |
| FE-03 | 2 | diagnostic-stream amplification | bug-505 |
| LNK-12 | 7 | project-name path traversal → arbitrary 0755 executable write (live) | bug-503 |
| LNK-13 | 7 | Windows PE has no ASLR | bug-504 |
| CRY-50 | 6 | Schannel `tls::write(List OF Byte)` uses String layout → OOB (box 2230) | bug-508 |
| DEC-50/51/53/54/55 | 5 | PNG/inflate/glyph/cmap/MML decompression bombs | bug-509 |
| DEC-01/02/03 | 5 | regex recursion/backtracking + json/csv amplification (DEC-03 live) | bug-510 |

(The DEC-* and OS-*/MEM-* cluster rows above bundle the individually-rated HIGH
findings named in each surface file; the per-ID severities and counts live there.)

### MEDIUM / LOW / NTH

Per-surface — see each `audit-3-*.md`. Notable MEDIUMs with their own bug doc:
SUP-01 (bug-189 augmented), SUP-02 (bug-489, live terminal-injection), SUP-03
(bug-490), SUP-04 (bug-491), CRY-01 (bug-511). REPO-50 (world-readable registry
DB, lead-confirmed) and REPO-04/51/52/55/56 are recorded for follow-up.

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
