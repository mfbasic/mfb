# goal-08: MFBASIC platform security review — code-grounded, trust-boundary audit

Last updated: 2026-09-03
Status: NOT STARTED (0 / 9 surfaces audited)

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

## Findings ledger

Update as findings are filed.

| ID | Surface | Title | Severity | Repro | Spike | Bug doc |
|----|---------|-------|----------|-------|-------|---------|
| _(none yet)_ | | | | | | |

Tallies: CRITICAL 0 · HIGH 0 · MEDIUM 0 · LOW 0 · NTH 0.

## Attack-surface map & progress

Audited by surface. Mark `- [x]` with a verdict when a surface is fully
covered (`clean`, or the finding ids filed). A file may appear under more than
one surface — the map is by trust boundary, not a partition. Directory paths
mean "every `.rs` under it".

**Surface 1 — Untrusted `.mfp` package decode + signature / IR verification** (`PKG-`)
_Untrusted party: author of a `.mfp` artifact on the dependency path._

- [ ] `src/binary_repr/{reader,sections,util,builder,writer,mod}.rs`
- [ ] `src/target/package_mfp/`
- [ ] `src/manifest/{entry,package,mod,json_edit}.rs`
- [ ] `src/target/shared/validate/`
- [ ] `src/cli/build/` (signature/hash gate at import/build)
- [ ] `src/cli/resolve.rs`, `src/resolver/packages.rs`

**Surface 2 — Language front-end (lexer / parser / resolver / rules / hir / monomorph / ir / optimizer input)** (`FE-`)
_Untrusted party: author of an arbitrary `.mfb` source file._

- [ ] `src/lexer.rs` (includes string-escape decoding), `src/numeric.rs`
- [ ] `src/ast/` (expr/stmt recursion depth)
- [ ] `src/resolver/`, `src/rules/`, `src/hir/`
- [ ] `src/monomorph/` (polymorphic-recursion instantiation)
- [ ] `src/ir/` (verify / lower), `src/optimizer/`
- [ ] `src/fmt.rs`, `src/audit/` (`mfb fmt` / `mfb audit` also consume
      arbitrary source)
- [ ] `src/unicode/` + calls into `third_party/utf8proc` (FFI boundary only)

**Surface 3 — Codegen & runtime memory safety (arena / collections / strings / engine / threads / canvas ring)** (`MEM-`)
_Untrusted party: whoever controls runtime inputs (sizes, strings, transfers, published scenes)._

- [ ] `src/codegen/memory/` (arena, data, marshal, owned, value)
- [ ] `src/codegen/collection/` (buffer, layout, assign, list, map, sort,
      search, compare)
- [ ] `src/codegen/string/` (repr, format, util, validate, unicode)
- [ ] `src/codegen/engine/` (builder, regalloc, validation, mir, operand)
- [ ] `src/codegen/{compiler/opt,cleanup,error,resource}/`
- [ ] `src/codegen/runtime/thread/` (cross-thread transfer; per-thread arena)
- [ ] `src/codegen/runtime/canvas/` (`metal.rs`, `vulkan.rs`, `shaders/`,
      scene ring; closed-flag texture-free rule — see `.ai/canvas-threading.md`)
- [ ] `src/codegen/builtins/{vector,bits,math,money}/` (SIMD / arithmetic
      bounds)
- [ ] `src/arch/`
- [ ] `src/target/{linux_aarch64,linux_x86_64,linux_riscv64,macos_aarch64,win_x86_64,linux_common}/` (per-target emit)
- [ ] `src/target/shared/{abi,lower,regmodel}.rs`, `src/target/shared/{nir,plan,runtime}/`

**Surface 4 — OS-touching runtime packages (fs / net / http / process / thread / term / io / os / app)** (`OS-`)
_Untrusted party: remote net/http peer; attacker-controlled paths/filenames/env; hostile terminal content; window-system input._

- [ ] `src/codegen/os/{ffi,process,socket,syscall}/`
- [ ] `src/codegen/io/{stdin,stdout,terminal}/`
- [ ] `src/codegen/builtins/{fs,os,io}/` (path handling, atomic writes, env)
- [ ] `src/codegen/builtins/{net,tcp,udp}/`
- [ ] `src/codegen/builtins/http/` (request/response/multipart/chunked/gzip
      parse — cross-ref Surface 5; header CRLF; SSRF; `respond_file`/
      `respond_path` traversal; `helper_limits.rs` bounds)
- [ ] `src/codegen/builtins/process/` (spawn/exec argument handling, signal
      disposition, fd inheritance)
- [ ] `src/codegen/builtins/thread/`
- [ ] `src/codegen/builtins/term/` + `src/codegen/term/` (ANSI/escape
      injection; `term::on` ISIG behavior)
- [ ] `src/codegen/builtins/app/` + `src/codegen/app/` (GUI events/input)
- [ ] `src/target/linux_gtk/{app_io,bootstrap,term_draw,mod}.rs`,
      `src/target/win_x86_64/app/`

**Surface 5 — Untrusted-data decoders in emitted programs** (`DEC-`)
_Untrusted party: author of any data file / byte stream a compiled program decodes._

- [ ] `src/codegen/builtins/encoding/` (base32/64, hex, percent, punycode,
      UTF-8/16/32, LEB128/varint, codepage, html un/escape)
- [ ] `src/codegen/builtins/json/`
- [ ] `src/codegen/builtins/csv/`
- [ ] `src/codegen/builtins/regex/` (pattern + subject DoS; `\x{...}` escapes)
- [ ] `src/codegen/builtins/canvas/helper_{png,inflate}.rs` (PNG + zlib
      inflate: decompression bombs, chunk-size arithmetic)
- [ ] `src/codegen/builtins/canvas/helper_{font,glyph,glyph_cache}.rs`,
      `gen_font*.rs`, `func_load_font.rs`, `func_load_image.rs` (untrusted
      font/image files)
- [ ] `src/codegen/builtins/audio/helper_mml_*.rs` (MML parse/synth),
      `func_play.rs`, `func_render.rs`
- [ ] `src/codegen/builtins/{strings,astrings,datetime}/` (parse entry points
      only: sizes/indices from untrusted text)
- [ ] http body decode helpers (cross-ref Surface 4)

**Surface 6 — Crypto / TLS / verification** (`CRY-`)
_Untrusted party: remote TLS peer; author of a signed `.mfp`._

- [ ] `src/codegen/builtins/crypto/` (AES-GCM seal/open, hash/HMAC/HKDF/
      PBKDF2, sign/verify, exchange, random, constant-time equal, `gen_cert`)
- [ ] `src/codegen/builtins/tls/` (handshake, cert-chain + name verification,
      downgrade, per-backend trust — see `.ai/net-tls.md`)
- [ ] Ed25519 `.mfp` signature path (cross-ref Surface 1)
- [ ] `repository/src/crypto.rs` (cross-ref Surface 8)

**Surface 7 — Custom linker & emitted-binary hardening (Mach-O / ELF / PE / AppImage)** (`LNK-`)
_Untrusted party: attacker exploiting an emitted binary at runtime._

- [ ] `src/os/linux/{link/,object.rs,appdir.rs,appimage/,flavor.rs}`
- [ ] `src/os/macos/{link/,object.rs,icon.rs}`
- [ ] `src/os/windows/{link/,object.rs}` (PE hardening flags — no prior audit)
- [ ] `src/os/{link_encode,object_plan,note}.rs`, `src/os/icon/`
- [ ] `src/codegen/link/{locator,thunk}/`

**Surface 8 — Package registry HTTP service (auth / transparency log / TUF metadata / blobs)** (`REPO-`)
_Untrusted party: any remote registry client (anonymous or token-holding)._

- [ ] `repository/src/server.rs` (all routes: auth/challenge/login, signing,
      log/*, keys/rotate, machines/*, tokens/*, packages/transfer/*,
      root/snapshot/timestamp.json, validate, publish, blob, search, rate
      limits)
- [ ] `repository/src/{validation,crypto,abi}.rs`
- [ ] `repository/src/{store,local,blobstore,package}.rs`
- [ ] `repository/src/{log,gc,backfill}.rs`
- [ ] `repository/src/web/`, `repository/src/{main,lib}.rs`
- [ ] `repository/docker-entrypoint.sh`, `repository/Dockerfile` (untrusted
      config only)

**Surface 9 — Supply chain: install / resolve / registry client (compiler side)** (`SUP-`)
_Untrusted party: malicious or MITM'd registry; spoofed dependency source._

- [ ] `src/cli/{pkg,repo,resolve,init}.rs`
- [ ] `src/manifest/{url,libraries}.rs`
- [ ] `repository/src/client.rs`
- [ ] cross-ref Surface 1 (`.mfp` verification) and Surface 6 (signature
      crypto)
