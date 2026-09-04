# goal-08: MFBASIC platform security review — code-grounded, trust-boundary audit

Last updated: 2026-09-03
Status: COMPLETE (9 / 9 surfaces audited) — audit series 3, executed via
`/follow-plan goal-08` in worktree `.claude/worktrees/P-goal-08`.

## Objective

Produce a **code-grounded security review** of the MFBASIC platform as it is
implemented today — the language front-end, `.mfp` IR/package decode &
verification, native codegen & runtime helpers, the custom Mach-O / ELF / PE
linker, the fs / net / http / thread / process / crypto / tls / term / canvas /
audio runtime packages, and the `mfb-repo` package registry service. This is
**not** a general bug hunt and **not** a spec-only read: every finding must be
verified against current source and, where practical, reproduced against a
built artifact (`target/debug/mfb`, a crafted `.mfb`/`.mfp`, or the running
registry).

Note: `planning/completed/goal-07-platform-security-review.md` authored this
same audit (series 3) on 2026-07-28 but was archived **unexecuted** — no
`audit-3-*` file exists (`ls planning planning/completed | grep audit-3` is
empty). This goal supersedes it; goal-07's surface map is stale (the runtime
codegen tree moved from `src/target/shared/code/**` to `src/codegen/**`, and
`src/builtins/*.rs` became per-package directories under
`src/codegen/builtins/`), so the map below was rebuilt from the current tree.

This is a **security** review: prioritize attacker-reachable impact —

- **Memory / resource safety** — OOB read/write, use-after-free, double-free,
  unchecked size arithmetic / integer overflow into an allocation, unbounded
  recursion or growth (native codegen + arena / collection / string / SIMD /
  vector runtime; the per-thread arena and cross-thread transfer seams).
- **Trust / auth bypass** — missing or forgeable signature/authentication,
  broken challenge/login or session/token handling, authorization gaps in the
  registry, confused-deputy paths, transparency-log or TUF-metadata forgery.
- **Injection** — command / path / format-string / log injection; SSRF from the
  HTTP / net client; CRLF into HTTP heads; ANSI / terminal-escape injection
  from the `term` / console backend.
- **Privilege escalation & sandbox escape** — crossing a boundary the design
  says should hold (author of an untrusted `.mfp` → code that runs at build or
  runtime; registry client → another owner's namespace; one thread → another's
  owned data; worker thread → graphics thread via the canvas scene ring).
- **Supply chain** — package/dependency substitution, unverified downloads,
  unpinned or spoofable sources, install-time or build-time code execution, a
  dropped-in `.mfp` trusted without signature / hash / IR re-verification.
- **Crypto / verification gaps** — missing signature/hash/cert verification,
  weak or misused primitives (Ed25519 / ECDSA / AES-GCM / TLS), predictable
  secrets, TOCTOU around verification, nonce / challenge reuse.
- **Attacker-triggerable DoS** — an untrusted party (remote peer, `.mfp`
  author, registry client, author of a hostile PNG/font/MML/regex/CSV input)
  can cheaply exhaust CPU, memory, disk, or handles, or wedge a handler.
- **Weak hardening** — missing exploit mitigations in emitted binaries
  (PIE/ASLR/NX/RELRO/stack canaries; on Windows: /DYNAMICBASE/NX/CFG), unsafe
  file permissions, secrets in logs/artifacts, information leaks across a
  boundary.

**Out of scope:** pure correctness, polish, or missing features — unless they
create a security-boundary failure. Do not file those here (they belong in a
`create-review` source-review goal, e.g. the `goal-06`/`goal-07` full-source
lineage).

## Scope

In-scope trees:

- `src/**` — compiler front-end, `.mfp` IR/package decode & verification,
  monomorph, native codegen & runtime helpers (`src/codegen/**`), custom
  linker (Mach-O / ELF / PE, `src/os/**`), CLI, per-target emit
  (`src/target/**`, `src/arch/**`).
- `repository/**` — the `mfb-repo` package registry HTTP service (auth,
  transparency log, TUF metadata, blob store, publish/validate, GC).

9 attack surfaces mapped below.

**Editable in this pass:** only `planning/` (audit files), `bugs/` (bug
documents), and `spikes/audit-3/` (MFB trigger programs). This is a
**find-and-document** pass — do not fix issues in the audited code here.

**Out of surface-scope** (with reason):

- `benchmark/`, `examples/`, `tests/`, `tools/`, `spikes/` — not
  attacker-reachable production code; a test fixture is in scope only if it
  masks a real boundary gap in `src/**` or `repository/**`.
- `packages/` — first-party MFB source packages compiled by the trusted build;
  in scope only where one implements a decoder for untrusted data.
- `src/docs/**`, `src/doc/**` — embedded documentation content; not executed.
- `third_party/utf8proc`, `third_party/unicode` — vendored/generated tables
  audited upstream. In scope only for *how MFBASIC calls into them* (untrusted
  sizes/pointers/indices crossing the boundary), not their internals.
- The compiler's own Rust dependencies (`sha2`, `tinyjson`, `unicode-*`,
  `rusqlite`/bundled SQLite, `image`/`icns` for build-host icon generation) —
  supply-chain-pinned via `Cargo.lock`; audit their *use*, not their source.

## Threat model — trust boundaries

For each surface, the untrusted party and what they must NOT be able to do.

- **Surface 1 — `.mfp` package decode + verification.** Untrusted party: author
  of a `.mfp` artifact on the dependency path (dropped in locally or fetched
  from a registry). Must not: corrupt memory / crash / execute code in the
  compiler via a malformed or hostile package, nor have unsigned / IR-tampered
  package contents accepted as trusted.
- **Surface 2 — Language front-end.** Untrusted party: author of an arbitrary
  `.mfb` source file the compiler is asked to build (or `mfb audit`/`mfb fmt`
  is asked to process). Must not: crash the compiler, exhaust memory/CPU
  (unbounded recursion/growth), or reach codegen with invalid IR.
- **Surface 3 — Codegen & runtime memory safety.** Untrusted party: whoever
  controls runtime inputs (sizes, strings, collection contents, cross-thread
  transfers, published canvas scenes) to a compiled program. Must not: cause
  OOB access, use-after-free/double-free, integer overflow into an allocation,
  or break an arena / collection / scene-ring ownership invariant in emitted
  code.
- **Surface 4 — OS-touching runtime packages (fs / net / http / process /
  thread / term / io / app).** Untrusted party: remote net/http peer;
  attacker-controlled paths/filenames/environment; hostile terminal output
  content; window-system input events. Must not: read/write outside intended
  paths, inject into a spawned command line, inject via CRLF/ANSI/format
  sinks, SSRF, or wedge/exhaust the process.
- **Surface 5 — Untrusted-data decoders in emitted programs (encoding / json /
  csv / regex / PNG / font / MML).** Untrusted party: author of any data file
  or byte stream a compiled program decodes (image, font, JSON/CSV document,
  regex subject, base-N/punycode/UTF payload, MML tune, gzip/multipart body).
  Must not: cause OOB/overflow in the decoder, non-terminating or
  super-linear blowup (decompression bombs, catastrophic regex, punycode
  overflow), or smuggle data past a validator (overlong UTF-8, header
  confusion).
- **Surface 6 — Crypto / TLS / verification.** Untrusted party: remote TLS
  peer; author of a signed `.mfp`. Must not: bypass signature/cert
  verification, force a downgrade, exploit a predictable secret/nonce, a
  non-constant-time compare on a secret, or a verification TOCTOU.
- **Surface 7 — Custom linker & emitted-binary hardening.** Untrusted party: an
  attacker exploiting an emitted binary at runtime (needs an in-program bug to
  chain against). Must not: find exploit mitigations (PIE/NX/RELRO/canary/CFG)
  silently absent from emitted Mach-O / ELF / PE / AppImage images.
- **Surface 8 — Package registry HTTP service.** Untrusted party: any remote
  registry client (anonymous or token-holding). Must not: bypass authz across
  owners/namespaces, forge signatures / transparency-log / TUF metadata, take
  over a name, poison the blob store, or cheaply exhaust the service.
- **Surface 9 — Supply chain: install / resolve / registry client.** Untrusted
  party: a malicious or MITM'd registry, or a spoofed dependency source. Must
  not: get an unverified / substituted / downgraded package accepted at
  install/build time, or run code at install time.

## Fix constraints (invariants a fix must respect)

Per `AGENTS.md` and the project memory:

- **No language-surface change.** A fix must not alter MFBASIC syntax or
  observable runtime semantics of a correct program.
- **No wire-format / public-API / registry-contract break.** The `.mfp` binary
  format, the registry HTTP contract (transparency-log/TUF metadata, name
  bindings), and package-index shapes are compatibility surfaces — a fix must
  not silently break them.
- **Never weaken a test or golden to pass.** Follow the `AGENTS.md` 4-point
  evidence gate before touching any behavioral test; `.ncode`/`.ncodesum`
  goldens are drift sentinels — a correct fix regenerates them, never the
  reverse.
- **Correctness over performance.** Never trade a leak or boundary check away
  for speed.
- **Production-ready only.** No stubs / placeholders / fallbacks as a "fix"; a
  fix is a real fix or it is filed as a bug for later.

These are constraints on *proposed fixes* recorded in findings — this pass
applies none of them, but a HIGH/CRITICAL finding's "Best fix" must respect
them.

## Prior work — re-verify before re-opening

This platform has been audited twice; both series are complete and archived
under `planning/completed/`. A third work order (goal-07) was written but never
run.

- **Audit 1** (`planning/completed/audit-1-*.md`, `goal-01`…`goal-04` lineage)
  — first trust-boundary pass. Four CRITICALs: PKG-01 (`.mfp` signature gate),
  PKG-02 (IR re-verification), MEM-01/02 (string size-overflow).
- **Audit 2** (`planning/completed/audit-2-*.md`,
  `goal-05-platform-security-review.md`; summary `audit-2-summary.md`) —
  second pass, 8 surfaces, completed 2026-07-14. Headline: all four audit-1
  CRITICALs and most HIGHs **fixed & re-verified**; no CRITICAL and no *new*
  HIGH found. Still-open items at that time were audit-1 carryovers filed as
  bugs: FE-02/FE-03 front-end DoS (bug-182/183), OS-01/OS-02 fs-helper
  defaults (bug-184/185), LNK-01 non-PIE Linux (bug-186); plus MEDIUMs
  REPO-12/13 (bug-188), SUP-02/03 (bug-189), OS-09 CRLF, LNK-08 (bug-187).
  Per-surface detail: `audit-2-{package-decode,frontend,codegen-memory,
  fs-net-thread,crypto-tls,linker-hardening,repository,supply-chain}.md`, plus
  `audit-unicode.md`.
- **goal-07** (`planning/completed/goal-07-platform-security-review.md`) —
  the unexecuted audit-3 work order; useful only for its threat-model prose.
  Its file paths predate the `src/codegen/**` restructure — do not trust them.

**New since audit-2** (audit these fresh — no prior security coverage): the
**Windows PE target** (`src/target/win_x86_64/**`, `src/os/windows/**`) and
its emitted-binary hardening; the **`linux_gtk` GUI target**
(`src/target/linux_gtk/**`) and the **app GUI package** with window-system
input (plan-13, plan-94 mouse); the **canvas GPU path** (Metal / Vulkan,
`src/codegen/runtime/canvas/**`) and its three-thread scene ring; **canvas
PNG/inflate and font loading** (`helper_png.rs`, `helper_inflate.rs`,
`helper_font*.rs` — untrusted image/font decode); the **audio package**
including the MML parser/synth (`src/codegen/builtins/audio/helper_mml_*`);
**http gzip / chunked / multipart / cookie handling** (plan-93); and the
**encoding / csv / regex / process** builtin packages.

Do not re-open a fixed item as a new finding without re-verifying against
*current* code — paths have moved wholesale since audit-2 (runtime helpers:
`src/target/shared/code/**` → `src/codegen/{memory,string,collection,io,os,
runtime,…}/**`; builtin surface: `src/builtins/<pkg>.rs` →
`src/codegen/builtins/<pkg>/`). Before calling any prior finding fixed or
open, check `bugs/`, `bugs/completed/`, and `bugs/skipped/` for its bug-NN. If
a prior finding is still open, reference its ID rather than duplicating the
analysis.

## Severity scale

- **CRITICAL** — attacker-reachable, high-impact, demonstrated (memory
  corruption with control, auth bypass, RCE, supply-chain substitution).
- **HIGH** — serious impact, reachable, strong evidence even if not fully
  weaponized.
- **MEDIUM** — real boundary weakness with limited impact or preconditions.
- **LOW** — defense-in-depth / latent; code path exists but no plausible
  trigger constructed.
- **NTH** — nice-to-have hardening.

Label only what you can support. **"Not demonstrated"** is an allowed, honest
label when a path exists but you could not exercise it — do not promote those
to CRITICAL/HIGH without evidence.

## Finding requirements

Each finding must include:

- **ID** (surface-prefixed: `PKG-`, `FE-`, `MEM-`, `OS-`, `DEC-`, `CRY-`,
  `LNK-`, `REPO-`, `SUP-`) and **severity** (scale above). Number fresh within
  audit-3; cross-reference audit-1/2 IDs where a finding is a re-open.
- **Title** and **location** — `path/file.rs:line` (or symbol) cited after a
  real source read.
- **Threat / impact** — who can trigger it and what breaks (confidentiality,
  integrity, availability, trust).
- **Mechanism** — why the code is wrong, not just that it feels risky.
- **Reproduction** — preferred: a minimal input/command against a built binary
  (`target/debug/mfb` on a crafted `.mfb`/`.mfp`, a compiled program on a
  crafted data file, or a request to the running `mfb-repo`); if pure
  decode/protocol/linker, a concrete byte/command repro. Record observed vs
  expected.
- **MFB trigger program (spike)** — where applicable: when the issue is
  triggerable from MFBASIC code or from data an MFB program feeds the system (a
  hostile `.mfb` source for front-end findings; a compiled program plus crafted
  PNG/font/MML/JSON/regex/HTTP input for runtime/decoder findings), check in a
  minimal trigger under `spikes/audit-3/`: a bare
  `spikes/audit-3/<finding-id>.mfb` when a single file suffices (the
  `bugs/repro/` shape), or a buildable project directory
  `spikes/audit-3/<finding-id>/` with `project.json` + `src/` when a manifest,
  package, or data file is needed (the `spikes/sN` shape, run via
  `mfb build spikes/audit-3/<finding-id> && ./spikes/audit-3/<finding-id>/build/mfb_project.out`).
  Record the exact command and observed vs expected next to the finding. A
  finding not expressible this way (crafted `.mfp` bytes, registry-side authz,
  a hostile TLS peer, emitted-binary hardening flags) states why no MFB spike
  is possible and relies on the byte/command repro above instead.
- **Best fix** — implementation-level, respecting the fix constraints above.
- **Non-goals** for that fix — what must stay the same.

## Outputs

1. **Audit files**, split by surface (next free audit series number = **3**;
   verified: no `audit-3-*` exists in `planning/` or `planning/completed/`):
   - `planning/audit-3-<surface>.md` per surface:
     `audit-3-package-decode.md`, `audit-3-frontend.md`,
     `audit-3-codegen-memory.md`, `audit-3-os-runtime.md`,
     `audit-3-decoders.md`, `audit-3-crypto-tls.md`,
     `audit-3-linker-hardening.md`, `audit-3-repository.md`,
     `audit-3-supply-chain.md`.
   - One index: `planning/audit-3-summary.md` with a master finding table (ID,
     severity, title, location, cross-links).
2. **Bug documents** via the **write-bug** skill (`bugs/bug-NN-<slug>.md` in
   its template) for every **CRITICAL** and **HIGH** finding (and **MEDIUM**
   when the fix is not small). Next free bug number: **489** (max across
   `bugs/`, `bugs/completed/`, `bugs/skipped/` is 488 — re-check at filing
   time; numbers race between sessions). Do not implement fixes here.
3. **MFB trigger spikes** under `spikes/audit-3/` — one per finding where
   applicable (see finding requirements), each referenced from its finding
   entry, the `audit-3-summary.md` table, and any bug doc. Add a
   `spikes/audit-3/README.md` table mapping finding ID → spike → one-line
   observed behavior, in the style of `spikes/README.md`.

## Method

1. **Map trust boundaries first** (done below; refine as you read).
2. **Fan out by surface** — parallel subagents are fine; each returns findings
   only, with `file:line` citations. Load the `mfbasic` MCP tools (`mfb_man`,
   `mfb_spec`) via `ToolSearch` for spec/behavior questions instead of
   guessing; read the matching `.ai/*.md` topic doc before each surface
   (`codegen-invariants`, `arch-abi`, `collections`, `resources-packages`,
   `canvas-threading`, `net-tls`, `testing-gates`).
3. **Re-verify every finding yourself** against current source before
   recording it — discard hallucinations and already-fixed items; check the
   prior-audit IDs and their bug-NNs.
4. **Write the audit files and summary; check in the trigger spikes; file bug
   docs** for CRITICAL/HIGH (and qualifying MEDIUM).
5. **Do not implement fixes in this pass.**

## Corrections

Divergences between this work order and reality, found while executing it.

- **2026-09-03 — audit-2's carryover bugs are no longer open.** The "Prior work"
  section above lists FE-02/FE-03, OS-01/OS-02, LNK-01, REPO-12/13, SUP-02/03
  and LNK-08 as still-open at audit-2. Measured at goal-08 start
  (`find bugs -name 'bug-18[2-9]-*.md'` + `grep -m1 -i '^Status' <each>`):

  | bug | location | Status line |
  |---|---|---|
  | bug-182 (FE-02 monomorph recursion) | `bugs/completed/` | Fixed |
  | bug-183 (FE-03 stmt-block recursion) | `bugs/completed/` | Fixed |
  | bug-184 (OS-01 world-writable mode) | `bugs/completed/` | Fixed |
  | bug-185 (OS-02 `net.accept` timeout) | `bugs/completed/` | Fixed |
  | bug-186 (LNK-01 non-PIE Linux) | `bugs/completed/` | Fixed (dynamic path; RELRO deferred to bug-187) |
  | bug-187 (LNK-08 writable constants) | `bugs/completed/` | Fixed on Linux (3 arches) + macOS aarch64 |
  | bug-188 (REPO-12/13 registry quota) | `bugs/completed/` | Fixed |
  | bug-189 (SUP-02/03 bootstrap/downgrade) | `bugs/skipped/` | **Partially Fixed — SUP-03 downgrade defense remaining** |

  So exactly one audit-2 carryover is still open (bug-189 / SUP-03), and it is
  in `bugs/skipped/`. Every audit-3 finding that re-opens one of the others must
  cite current source, not the audit-2 text.

- **2026-09-03 — next free bug number confirmed as 489.** Measured:
  `ls bugs bugs/completed bugs/skipped | grep -oE 'bug-[0-9]+' | sort -n | tail`
  gives 488, and `git log --all --grep='bug-4[89][0-9]'` shows no bug-489+ on
  any branch. The work order's "next free: 489" holds.

## Findings ledger

Update as findings are filed.

| ID | Surface | Title | Severity | Repro | Spike | Bug doc |
|----|---------|-------|----------|-------|-------|---------|
| [PKG-01](audit-3-package-decode.md) | 1 | Signature gate and codegen-feeding decode are separate, unsynchronised reads of the same path (TOCTOU) | LOW | structural (5 `fs::read` sites; no signature call on the decode path) | n/a — not MFB-expressible | — |
| [SUP-01](audit-3-supply-chain.md) | 9 | `/index` version list unsigned; `snapshot.indexHash` decoded then discarded (downgrade) | MEDIUM | code (index_hash dead code) | n/a — registry protocol | bug-189 (augmented) |
| [SUP-02](audit-3-supply-chain.md) | 9 | Registry error string renders unsanitized → forged `[Verified]` line | MEDIUM | **live** (`spikes/audit-3/SUP-02/`) | SUP-02 (harness, not `.mfb`) | bug-489 |
| [SUP-03](audit-3-supply-chain.md) | 9 | Cross-origin 307/308 redirect re-posts the body-borne session token | MEDIUM | code (guard + 307 body + token-in-body) | n/a — network boundary | bug-490 |
| [SUP-04](audit-3-supply-chain.md) | 9 | `pkg install` never binds the fetched blob to the lock's ident/version | MEDIUM | code (3 lossy callers) | n/a — registry protocol | bug-491 |
| [SUP-05](audit-3-supply-chain.md) | 9 | `put_blob` error path reads the body uncapped (incomplete bug-276 R3) | LOW | code | n/a | bug-276 (note) |
| [SUP-06](audit-3-supply-chain.md) | 9 | Key/session files chmod'd after create; symlink-followed; `~/.mfb` unrestricted | LOW | code | n/a | — |
| [SUP-07](audit-3-supply-chain.md) | 9 | Redirect IP guard misses 6to4 / NAT64 / `0.0.0.0/8` | NTH | code | n/a | — |
| [SUP-08](audit-3-supply-chain.md) | 9 | Registry-supplied `hash` interpolated into blob URL with no hex check | NTH | code | n/a | — |
| [REPO-01](audit-3-repository.md) | 8 | Scoped publish token self-escalates to a permanent unscoped auth key | HIGH | **live** (`spikes/audit-3/repository-authz/`) | harness (not `.mfb`) | bug-492 |
| [REPO-02](audit-3-repository.md) | 8 | `/machines/revoke` accepts an auth-key challenge → account lockout | HIGH | **live** | harness | bug-493 |
| [REPO-03](audit-3-repository.md) | 8 | `/release-state`+`/signing` authorize on ident prefix, not owner | HIGH | **live** | harness | bug-494 |
| [REPO-04](audit-3-repository.md) | 8 | Unauth 64 MiB body buffered before auth; no concurrency cap/timeout | MEDIUM | reproduced (RSS) | n/a | — |
| [REPO-50](audit-3-repository.md) | 8 | Registry SQLite DB created world-readable (holds signing key) | MEDIUM | **lead-confirmed** (0644) | n/a | — |
| [REPO-51](audit-3-repository.md) | 8 | Case-insensitive `LIKE` → inclusion-proof pointer resolves to another package | MEDIUM | SQL-level | n/a | — |
| [REPO-52/55/56/06/07](audit-3-repository.md) | 8 | Storage/CPU exhaustion + per-IP bucket collapse behind proxy | MEDIUM | code/repro | n/a | — |
| [REPO-05/08/09/10/53/54/57](audit-3-repository.md) | 8 | Rate-limit/expiry/blob-rehash/log-chain/key-perm gaps | LOW | code | expired: harness | — |
| [REPO-58/59/60](audit-3-repository.md) | 8 | root version unchecked · NUL-sep signed msgs · i64→usize widen | NTH | code | n/a | — |
| [OS-50](audit-3-os-runtime.md) | 4 | `tcp/tls/udp` write of a String-from-call → byte-list lowering → remote peer-controlled OOB read | **CRITICAL** | **live** (`spikes/audit-3/OS-50/`) | OS-50 | bug-497 |
| [OS-01/04](audit-3-os-runtime.md) | 4 | Spawned children inherit fds/sockets (no CLOEXEC; Windows bInheritHandles) | HIGH | code (flag words) | — | bug-499 |
| [OS-02](audit-3-os-runtime.md) | 4 | env-replace clear loop infinite-loops on an `environ` entry with no `=` | HIGH | code + agent 17 GB | — | bug-500 |
| [OS-53/54/55](audit-3-os-runtime.md) | 4 | HTTP CRLF injection (method + response) + request-smuggling toolbox | HIGH | code + OS-54 demo | — | bug-506 |
| [OS-51/52/56](audit-3-os-runtime.md) | 4 | HTTP server DoS: chunk-abort, no timeout (slowloris), quadratic head | HIGH | code + measured | — | bug-507 |
| [OS-03/05..13/24/25/57/58/59](audit-3-os-runtime.md) | 4 | binary-planting, ANSI injection, TOCTOU, GUI-input, url `\` — see file | MEDIUM | code/repro | — | — |
| [MEM-11](audit-3-codegen-memory.md) | 3 | Bounds-check elision stale-len → OOB heap read (all -O) | HIGH | **live** (`spikes/audit-3/MEM-11/`) | MEM-11 | bug-495 |
| [MEM-12](audit-3-codegen-memory.md) | 3 | `g = <op>(g, f())` reads freed block (UAF) via `&`/append | HIGH | **live** (`spikes/audit-3/MEM-12/`) | MEM-12 | bug-496 |
| [MEM-70](audit-3-codegen-memory.md) | 3 | `thread::send` allocates on peer arena unlocked → heap corruption | HIGH | **live** (`spikes/audit-3/MEM-70/`) | MEM-70 | bug-498 |
| [MEM-40](audit-3-codegen-memory.md) | 3 | Win64 entry seed scratch aliases stdin buffer slot → arbitrary ptr R/W | HIGH | **codegen-verified** (`spikes/audit-3/MEM-40/`) | MEM-40 | bug-512 |
| [MEM-41/71/72/13](audit-3-codegen-memory.md) | 3 | trampoline home-space · scene-ring/setBytes leaks · copy size helpers | MEDIUM | code/measured | — | — |
| [DEC-50/51/53/54/55](audit-3-decoders.md) | 5 | PNG/inflate/glyph/cmap/MML decompression bombs (no cap) | HIGH | crafted-file measured | DEC-50/51/55 | bug-509 |
| [DEC-01/02/03](audit-3-decoders.md) | 5 | regex recursion/backtracking + json/csv collection amplification | HIGH | DEC-03 **live** (1.2MB→1.05GB) | DEC-03 | bug-510 |
| [DEC-04/05/07/52](audit-3-decoders.md) | 5 | grapheme-tokenized json · punycode O(n²) · signed hex escape · quadratic IDAT | MEDIUM | code/measured | — | — |
| [FE-01](audit-3-frontend.md) | 2 | operator-chain compiler stack overflow (SIGABRT) | HIGH | **live** (`spikes/audit-3/FE-01/`) | FE-01 | bug-501 |
| [FE-02](audit-3-frontend.md) | 2 | `mfb fmt` quadratic blowup + non-atomic source overwrite | HIGH | agent 17 GB | — | bug-502 |
| [FE-03](audit-3-frontend.md) | 2 | diagnostic-stream amplification (240 KB → 10.4 GB stderr) | HIGH | agent measured | — | bug-505 |
| [FE-04/05/06 · FE-50/51](audit-3-frontend.md) | 2 | diag terminal injection · symlink read · audit O(N²) · verify/opt O(n²) | MEDIUM | code/measured | — | — |
| [CRY-50](audit-3-crypto-tls.md) | 6 | Schannel `tls::write(List OF Byte)` uses String layout → OOB, Windows remote crash | HIGH | box-2230 demo | — | bug-508 |
| [CRY-01](audit-3-crypto-tls.md) | 6 | X25519 ladder branches on private-scalar bit (timing side-channel) | MEDIUM | code | — | bug-511 |
| [CRY-51/52/53](audit-3-crypto-tls.md) | 6 | TLS-floor not enforced (Schannel/macOS) · audit misses positional allowSelfSigned · Win key-container | MEDIUM | code/demo | — | — |
| [LNK-12](audit-3-linker-hardening.md) | 7 | `project.json` name path-traversal → arbitrary 0755 executable write | HIGH | **live** (`spikes/audit-3/LNK-12/`) | LNK-12 | bug-503 |
| [LNK-13](audit-3-linker-hardening.md) | 7 | Windows PE has no ASLR (RELOCS_STRIPPED, no .reloc, DYNAMIC_BASE clear) | HIGH | code (header bytes) | — | bug-504 |
| [LNK-14/15/16](audit-3-linker-hardening.md) | 7 | PE no CFG/load-config · 633 KB writable const tables · Windows LINK path overrun | MEDIUM | code | — | — |

Tallies (CRITICAL + HIGH enumerated; MEDIUM/LOW/NTH are per-surface aggregates —
see each `audit-3-*.md`):
**CRITICAL 1 · HIGH 28.** MEDIUM ≈ 40 · LOW ≈ 22 · NTH ≈ 10 across the nine files.

New bug docs filed: **bug-489 … bug-512** (24), plus bug-189 augmented and a note
on bug-276. Every CRITICAL and HIGH has a bug doc; MEDIUMs with a larger fix are
recorded in their surface file for follow-up.

## Attack-surface map & progress

Audited by surface. Mark `- [x]` with a verdict when a surface is fully
covered (`clean`, or the finding ids filed). A file may appear under more than
one surface — the map is by trust boundary, not a partition. Directory paths
mean "every `.rs` under it".

**Surface 1 — Untrusted `.mfp` package decode + signature / IR verification** (`PKG-`)
_Untrusted party: author of a `.mfp` artifact on the dependency path._

- [x] `src/binary_repr/{reader,sections,util,builder,writer,mod}.rs`
- [x] `src/target/package_mfp/`
- [x] `src/manifest/{entry,package,mod,json_edit}.rs`
- [x] `src/target/shared/validate/`
- [x] `src/cli/build/` (signature/hash gate at import/build) — **PKG-01 (LOW)**;
      audit-1 PKG-01 re-verified fixed. `audit-3-package-decode.md` Q1.
- [x] `src/cli/resolve.rs`, `src/resolver/packages.rs` — `resolver/packages.rs`
      clean; `cli/resolve.rs` (1740 lines) in the gap pass.

**Surface 2 — Language front-end (lexer / parser / resolver / rules / hir / monomorph / ir / optimizer input)** (`FE-`)
_Untrusted party: author of an arbitrary `.mfb` source file._

- [x] `src/lexer.rs` (includes string-escape decoding), `src/numeric.rs`
- [x] `src/ast/` (expr/stmt recursion depth)
- [x] `src/resolver/`, `src/rules/`, `src/hir/`
- [x] `src/monomorph/` (polymorphic-recursion instantiation)
- [x] `src/ir/` (verify / lower), `src/optimizer/`
- [x] `src/fmt.rs`, `src/audit/` (`mfb fmt` / `mfb audit` also consume
      arbitrary source)
- [x] `src/unicode/` + calls into `third_party/utf8proc` (FFI boundary only)

**Surface 3 — Codegen & runtime memory safety (arena / collections / strings / engine / threads / canvas ring)** (`MEM-`)
_Untrusted party: whoever controls runtime inputs (sizes, strings, transfers, published scenes)._

- [x] `src/codegen/memory/` (arena, data, marshal, owned, value)
- [x] `src/codegen/collection/` (buffer, layout, assign, list, map, sort,
      search, compare)
- [x] `src/codegen/string/` (repr, format, util, validate, unicode)
- [x] `src/codegen/engine/` (builder, regalloc, validation, mir, operand)
- [x] `src/codegen/{compiler/opt,cleanup,error,resource}/`
- [x] `src/codegen/runtime/thread/` (cross-thread transfer; per-thread arena)
- [x] `src/codegen/runtime/canvas/` (`metal.rs`, `vulkan.rs`, `shaders/`,
      scene ring; closed-flag texture-free rule — see `.ai/canvas-threading.md`)
- [x] `src/codegen/builtins/{vector,bits,math,money}/` (SIMD / arithmetic
      bounds)
- [x] `src/arch/`
- [x] `src/target/{linux_aarch64,linux_x86_64,linux_riscv64,macos_aarch64,win_x86_64,linux_common}/` (per-target emit)
- [x] `src/target/shared/{abi,lower,regmodel}.rs`, `src/target/shared/{nir,plan,runtime}/`

**Surface 4 — OS-touching runtime packages (fs / net / http / process / thread / term / io / os / app)** (`OS-`)
_Untrusted party: remote net/http peer; attacker-controlled paths/filenames/env; hostile terminal content; window-system input._

- [x] `src/codegen/os/{ffi,process,socket,syscall}/`
- [x] `src/codegen/io/{stdin,stdout,terminal}/`
- [x] `src/codegen/builtins/{fs,os,io}/` (path handling, atomic writes, env)
- [x] `src/codegen/builtins/{net,tcp,udp}/`
- [x] `src/codegen/builtins/http/` (request/response/multipart/chunked/gzip
      parse — cross-ref Surface 5; header CRLF; SSRF; `respond_file`/
      `respond_path` traversal; `helper_limits.rs` bounds)
- [x] `src/codegen/builtins/process/` (spawn/exec argument handling, signal
      disposition, fd inheritance)
- [x] `src/codegen/builtins/thread/`
- [x] `src/codegen/builtins/term/` + `src/codegen/term/` (ANSI/escape
      injection; `term::on` ISIG behavior)
- [x] `src/codegen/builtins/app/` + `src/codegen/app/` (GUI events/input)
- [x] `src/target/linux_gtk/{app_io,bootstrap,term_draw,mod}.rs`,
      `src/target/win_x86_64/app/`

**Surface 5 — Untrusted-data decoders in emitted programs** (`DEC-`)
_Untrusted party: author of any data file / byte stream a compiled program decodes._

- [x] `src/codegen/builtins/encoding/` (base32/64, hex, percent, punycode,
      UTF-8/16/32, LEB128/varint, codepage, html un/escape)
- [x] `src/codegen/builtins/json/`
- [x] `src/codegen/builtins/csv/`
- [x] `src/codegen/builtins/regex/` (pattern + subject DoS; `\x{...}` escapes)
- [x] `src/codegen/builtins/canvas/helper_{png,inflate}.rs` (PNG + zlib
      inflate: decompression bombs, chunk-size arithmetic)
- [x] `src/codegen/builtins/canvas/helper_{font,glyph,glyph_cache}.rs`,
      `gen_font*.rs`, `func_load_font.rs`, `func_load_image.rs` (untrusted
      font/image files)
- [x] `src/codegen/builtins/audio/helper_mml_*.rs` (MML parse/synth),
      `func_play.rs`, `func_render.rs`
- [x] `src/codegen/builtins/{strings,astrings,datetime}/` (parse entry points
      only: sizes/indices from untrusted text)
- [x] http body decode helpers (cross-ref Surface 4)

**Surface 6 — Crypto / TLS / verification** (`CRY-`)
_Untrusted party: remote TLS peer; author of a signed `.mfp`._

- [x] `src/codegen/builtins/crypto/` (AES-GCM seal/open, hash/HMAC/HKDF/
      PBKDF2, sign/verify, exchange, random, constant-time equal, `gen_cert`)
- [x] `src/codegen/builtins/tls/` (handshake, cert-chain + name verification,
      downgrade, per-backend trust — see `.ai/net-tls.md`)
- [x] Ed25519 `.mfp` signature path (cross-ref Surface 1)
- [x] `repository/src/crypto.rs` (cross-ref Surface 8)

**Surface 7 — Custom linker & emitted-binary hardening (Mach-O / ELF / PE / AppImage)** (`LNK-`)
_Untrusted party: attacker exploiting an emitted binary at runtime._

- [x] `src/os/linux/{link/,object.rs,appdir.rs,appimage/,flavor.rs}`
- [x] `src/os/macos/{link/,object.rs,icon.rs}`
- [x] `src/os/windows/{link/,object.rs}` (PE hardening flags — no prior audit)
- [x] `src/os/{link_encode,object_plan,note}.rs`, `src/os/icon/`
- [x] `src/codegen/link/{locator,thunk}/`

**Surface 8 — Package registry HTTP service (auth / transparency log / TUF metadata / blobs)** (`REPO-`)
_Untrusted party: any remote registry client (anonymous or token-holding)._

- [x] `repository/src/server.rs` — REPO-01/02/03 (HIGH), REPO-04/06/07/52/55/56
      (MED), REPO-05/08/09/10 (LOW), REPO-58 (NTH). Read by route.
- [x] `repository/src/{validation,crypto,abi}.rs` — REPO-59 (NTH); crypto core
      fail-closed.
- [x] `repository/src/{store,local,blobstore,package}.rs` — REPO-50 (world-
      readable DB, lead-confirmed), REPO-51 (MED), REPO-53/54/57 (LOW).
- [x] `repository/src/{log,gc,backfill}.rs` — log inclusion/consistency
      fail-closed; REPO-53/54.
- [x] `repository/src/web/`, `repository/src/{main,lib}.rs` — main/lib read;
      `web/` skimmed (flag if a web-XSS pass is wanted).
- [x] `repository/docker-entrypoint.sh`, `repository/Dockerfile` — no default
      secret / root-run finding beyond REPO-06 (Fly proxy collapses per-IP).

Verdict: **3 HIGH (bug-492/493/494, all lead-reproduced live), 7 MEDIUM, 6 LOW,
2 NTH.** See `audit-3-repository.md`; harness `spikes/audit-3/repository-authz/`.

**Surface 9 — Supply chain: install / resolve / registry client (compiler side)** (`SUP-`)
_Untrusted party: malicious or MITM'd registry; spoofed dependency source._

- [x] `src/cli/{pkg,repo,resolve}.rs` — SUP-01..08. `init.rs` grepped (no
      network path). `pkg.rs` publish/remove/doc halves skimmed, not read.
- [x] `src/manifest/{url,libraries}.rs` — url.rs full; libraries.rs bare-name +
      vendor-hash path (locator body not read).
- [x] `repository/src/client.rs` — read 1-1520 (all production code).
- [x] cross-ref Surface 1 (`.mfp` verification, PKG-01) and Surface 6/8
      (signature crypto) — done.

Verdict: **SUP-01..04 MEDIUM, SUP-05/06 LOW, SUP-07/08 NTH.** See
`audit-3-supply-chain.md`. bug-489/490/491 filed; bug-189 augmented.
