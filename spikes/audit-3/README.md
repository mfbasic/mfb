# audit-3 trigger spikes

Minimal triggers for the audit-3 platform security review
(`planning/goal-08-platform-security-review.md`). Each reproduces one finding
against a built artifact; the per-spike `README.md` records the exact command and
observed-vs-expected. Findings whose trigger is not an MFB program (a registry
HTTP exploit, a crafted `.mfp`, an emitted-binary flag) either ship a non-`.mfb`
harness here or rely on the byte/command repro in the finding, as the work order
allows.

| Spike | Finding | Severity | One-line observed behavior |
|---|---|---|---|
| `OS-50/` | OS-50 / bug-497 | **CRITICAL** | `tcp::write(sock, f(str))`: 22-byte request → **1024 bytes of process memory** returned to the peer (peer-chosen length) |
| `MEM-11/` | MEM-11 / bug-495 | HIGH | bounds-check elision on a stale length → `out=24` from an 8→1 list, 14 heap words leaked (all `-O`) |
| `MEM-12/` | MEM-12 / bug-496 | HIGH | `GS & same()` where `same` reassigns `GS` → `len=4` not `12` (freed-block read) |
| `MEM-70/` | MEM-70 / bug-498 | HIGH | parent→worker `thread::send` loop → **SIGSEGV 3/3** in `_mfb_arena_alloc` |
| `MEM-40/` | MEM-40 / bug-512 | HIGH | windows-x86_64 ncode: entry seed write and stdin-buffer read both at `arena+3736` (linux control clean) |
| `FE-01/` | FE-01 / bug-501 | HIGH | 40 KB `1+1+…` chain → compiler "stack overflow, aborting" (SIGABRT) |
| `LNK-12/` | LNK-12 / bug-503 | HIGH | project named `../…/evil` → 0755 Mach-O written **outside** the project tree |
| `DEC-03/` | DEC-03 / bug-510 | HIGH | 1.2 MB JSON array → **~1.05 GB RSS** (~875× amplification) |
| `DEC-50/` | DEC-50 / bug-509 | HIGH | 69-byte PNG (IHDR 4000×4000) → 4.95 GB before `ErrBadImageFile` |
| `DEC-51/` | DEC-51 / bug-509 | HIGH | 389 KB zlib bomb → 25 GB, decode reports success |
| `DEC-55/` | DEC-55 / bug-509 | HIGH | 15-char MML tune with a huge repeat count → 38 GB (killed) |
| `SUP-02/` | SUP-02 / bug-489 | MEDIUM | registry error string with ESC/CR/U+202E renders as a forged `[Verified]` line (not a `.mfb` — HTTP harness) |
| `repository-authz/` | REPO-01/02/03 / bug-492/493/494 | HIGH | scoped token → unscoped signing post-revoke · auth-key revokes auth-key · former owner keeps yank/attest (not `.mfb` — HTTP harness) |

`gen/` holds the crafted-input generators for the media spikes
(`mkpng.py`, `mkbomb.py`, `mkchunks.py`, `mkfont.py`, `cmap12.py`, `setup.py`).
Findings without a spike here (they are not MFB-expressible and rely on the
byte/command repro in their finding): SUP-01/03/04..08, REPO-04..60,
OS-51..59, CRY-50/51/52/53, LNK-13/14/15/16, FE-02/03/…, and the MEDIUM/LOW tails.
