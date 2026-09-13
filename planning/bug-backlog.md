# Open bug backlog — triage and work order

Last updated: 2026-09-13
Open bugs: **12** (`find bugs -maxdepth 1 -name 'bug-*.md' | wc -l`)

## 2026-09-12 — integration rounds and what they taught

### Landed

| Bug | Sev | On main | Outcome |
|---|---|---|---|
| 595 | LOW | `4dbf42c03` | a `STATE` type name resolves like every other type position: binding, parameter, return and `LINK` STATE clauses were never resolved (unlocated `TYPE_STATE_INVALID`, a mismatch printing `pkg.Name`, or no diagnostic at all); four RED/GREEN fixtures + a qualified-import positive |
| 599 | MED | `c70c6d5d8` | PARTIAL: the address builders free their `inet_ntop` buffer and `net::lookup` its temp record (−38% growth per lookup, −57% per `tcp::localAddress`) — **stays OPEN**: the list/record/host `String` still have no owner (decision below); filed bug-601 (HIGH, the aliasing crash that blocks a drop) |
| 564 | MED | `8dbefee94` | sighting 2 fixed: macOS tls handlers publish the error domain before the gate with `stlr`, `tls::write` loads its gates with `ldar` (matched pair 19/600 → 0/600) — **stays OPEN** for the half-close write decision below |
| 600 | MED | `7bb602d1d` | a timed-out test run kills its program's whole process group, not just the program — a hung RED run's pipeline had orphaned and spun ~5 days (found when the user spotted two leftover probes, since killed) |
| 593 | MED | `8413676c0` | an inline `TRAP`'s `Result` wrapper over a non-flat `T` has an owner (RSS pins 17 → 313 MB and 13 → 313 MB on base, flat fixed); residual closed-record growth is plan-52-B by design; the `List OF net::Address` leak it uncovered is bug-599 |
| 472 | MED | `17c424988` | the man-example gate is its own CI job (`man-examples`); merged-tree Linux sweep 1029 examples / 0 failed / exit 0; it found bugs 595, 596 and 597 on its first run |
| 596 | MED | `3b8e0f31c` | a named call may omit an overloaded builtin's trailing DEFAULTED parameters (`tls::connect`'s documented form built again); the first cut re-opened bug-349 and was reworked to fail closed before landing |
| 597 | MED | `e633cefff` | `tls::listen("")` binds every interface on Linux and Windows (bug-113's `getaddrinfo(NULL, NULL)` defect in the TLS helpers); runtime-proven on boxes 2223 and 2230 |
| 594 | MED | round 4 `b5c8757a4` | macOS `term::drawText` advances the column for a control character |
| 592 | MED | round 3 `6ca7e8b8e` | an unbound `collections::get`/`getOr` `String` element has an owner (`os.prog` joined the fresh-result census after a lowering audit) |
| 540 | MED | round 3 `6ca7e8b8e` | WIN-04 only: Windows `term::drawText` shares `io::write`'s cluster walk — **stays OPEN** (WIN-02/03 decision); the Windows smoke run on box 2230 passed (`e791bc158`) |
| 576 | MED | round 2 `2f40cf6b4` | an unbound runtime-helper `String` result has an owner |
| 590 | HIGH | round 2 `2f40cf6b4` | a non-finite `Float` from a builtin call no longer escapes the observation boundary |
| 575 | MED | round 2 `2f40cf6b4` | the `String` argument every `tls::` call marshals is released |
| 591 | MED | `8361ec0c6` | a multi-overload man page renders each overload's own parameters |
| — | — | `5c1e09c33` | cleared the four deny-level clippy errors on main (CI never runs clippy) |
| 552 | LOW | `d83714554` | all three Level-2 global optimizer rows fire; full suite on main 161 targets, 0 failed |

Round 2 was verified on the merged tree, not per branch: artifact gate 1431 / 1597 /
2009, 0 diffs; merged `rt_scope_drop_leaks` 98 passed; merged release unit suite
4132 passed, 0 failed.

Round 3 was verified the same way: artifact gate 1431 / 1597 / 2009, 0 diffs; merged
release unit suite 4137 passed, 0 failed; merged `rt_scope_drop_leaks` 110 passed.

Round 4 (594) was verified the same way: artifact gate 0 diffs; merged release unit suite
4137 passed; `cli_macos_app_term_draw_text` 3 passed (the gate is blind to app mode).

**In flight:** nothing. No agents are running. Every bug worked this stretch has landed;
the ones that stay open (540, 564, 601) are blocked only on the owner decisions below
(599 was fixed by plan-132; 602 was tested and closed as not a defect). The remaining
open docs (484, 536) are large planned work, not quick fixes. 520 was re-scoped on
2026-09-13 — see **datetime** below.

**Filed this stretch:** 601 (HIGH — a `MUT` copy of a non-flat list aliases its source and
an in-place `append` then segfaults; found while fixing 599), 600 (a timed-out test run
orphaned its program's children — two probes spun ~5 days), 599 (a `List OF net::Address` is
never freed; found while fixing 593), 597 and 596 (both found by the 472 gate's first sweep — a Linux/
Windows `tls::listen("")` runtime defect and a bug-477 named-argument regression), 595
(found by the same sweep: a `STATE` type name is never resolved — unlocated
`TYPE_STATE_INVALID`, internal `pkg.Name` spelling in a mismatch message), 593 (a
failing runtime-helper call grows a flat block per call — two bugs measured it and
neither filed it), 594 (macOS `drawText` column), 592.

### Open decisions — these need the owner, not an agent

- **bug-564, the one open finding — DECIDED 2026-09-12: raise like every other platform.**
  On macOS, once `tls::read` has reported the peer closed, later `tls::write` calls never
  raise (20 000 × 64 KiB "complete" in under a second; main behaves the same). The owner's
  ruling: every function works the same on every platform, so that write must fail with
  `ErrConnectionClosed`, exactly as `mfb spec stdlib transports` §17 already requires
  ("on `tcp` and on `tls` alike, and on every target"; a first write may still be accepted
  locally, a later one raises). Half-close is not exposed. Not started; the fix is on the
  macOS Network.framework side. Linux and Windows have not been measured on this exact
  sequence, so the fix should confirm them too. Sighting 2 itself is fixed
  (`stlr`/`ldar`, matched pair 19/600 → 0/600).
- ~~**bug-599 / bug-601 — DECIDED 2026-09-12: FLATTEN.**~~ **Done by plan-132
  (2026-09-13).** `net::Address`, `udp::Datagram`, `net::PingResult` and
  `audio::AudioDevice` are built through the record marshaller onto the ordinary flat
  layout, every native reader rebases, and `is_pointer_string_record` is deleted. bug-599 is
  fixed and archived (a looped `net::lookup` held 1.2 → 1.4 MB over 200k → 400k iterations,
  was 99.8 → 198.3 MB). bug-601 **stays OPEN** for its recursive-type row only (`List OF
  Tree`, still aliases; rides on bug-536 shape C). The `Error`/`ErrorLoc` question
  (bug-602) is **closed, not a defect** (2026-09-13): both are ordinary flat records, every
  second-binding shape copies (bug-601's three list shapes on a `List OF Error` included, on
  the compiler before plan-132 as well), and `an_error_value_copy_is_independent_of_its_source`
  pins it.
- **bug-593's residual**: a successful or failing `RES` call still grows by the closed
  resource record plan-52-B deliberately never frees (aliases read its closed flag).
  Reclaiming it moves a lifetime.

- ~~**bug-581 Phase 2**~~ — **DECIDED 2026-09-12: won't fix; bug closed.** Binding a
  package index to signed snapshot state would defeat a registry that omits a version, but
  the owner ruled it not fixable in practice: "you have to trust the registry at some point,
  or dont use it." Phase 1 (route binding, `ed87c111a`) stays.
- **`collections::sum` (bug-590) and `set` (bug-563) declared errors**: whether an error
  raised at the CALLER's observation boundary belongs in the callee's `errors` list. The
  list drives inline-`TRAP` fallibility, so it moves more than a man page.
- **bug-540 WIN-02/03**: proving a Windows resize needs a test-only `term::` resize hook —
  product surface. WIN-04 did not need it.
- **A clippy CI job**: four deny-level errors reached main because nothing runs clippy.

### What the integration rounds taught

- **Integrate stale-base branches in rounds.** Each agent branch regenerated goldens on its
  own old base; every merge then conflicted on goldens a later commit also moved. One
  integration worktree per round — merge the batch, rebuild once, regenerate once under
  bash, gate once — resolved it. Both rounds changed EXACTLY the conflicted placeholder
  goldens and nothing else, which is the containment proof a post-merge regen can give.
- **Verify where you can still land.** A long suite in the shared main checkout made main
  un-advanceable for hours: landing writes goldens under it. Run long verification in a
  worktree.
- **The merged unit suite catches what per-branch suites cannot.** bug-576's census was
  correct on its branch and failed on the merged tree, because `os.prog` reached main from a
  peer after 576 branched. The fix was an AUDIT (does `os.prog` really allocate in the
  caller's arena?) before touching the list, not adding the name to make it green.
- **A landing gate must fail closed on the unexpected.** It aborted once on a peer's
  spec-markdown commit it had not anticipated; inspecting it before landing is the point.
- **zsh does not word-split an unquoted variable.** It silently broke a multi-path
  `git checkout` and a file loop this session. Run multi-path shell under bash.

## 2026-09-12 — compiler pass (after the repository intake)

| Bug | Sev | Commit | Outcome |
|---|---|---|---|
| 550 | MED | `2203554bb` | a generic builtin parameter now gets an expected type, so `append([], x)` builds |
| 564 | MED | `d77c3ced0` | `tls::close` drains to `cancelled` — a real use-after-munmap; bug stays OPEN for sighting 2 |
| 563 | MED | `ceeefcf24` | `collections::get`'s merged error union split per overload |
| 552 | LOW | `d83714554` | all three Level-2 global rows fire for the first time |
| 559 | LOW | `3173773c4` | `civil`/`addDays` finally demonstrate a DST transition |
| 576 | MED | `fbddba488` (landed in round 2 `2f40cf6b4`) | an unbound runtime-helper `String` now has an owner |
| 548 | LOW | archived | both paths settled; deletion independently re-confirmed |

**Filed after reproducing** (see the duplicate-filing lesson below): **590 HIGH**
(a non-finite `Float` from a builtin call escapes every observation boundary),
**591 MED** (a multi-overload man page renders only overload 1's parameters),
**592 MED** (an unbound `collections::getOr` `String` element has no owner).

### Findings worth carrying forward

- **Search for the DEFECT, not the number, before filing.** Three bugs
  (587/588/589) were filed as "new" out of bug-536's "recorded rather than filed"
  list. That note was six days stale — the defects had been filed as 560/561/562
  the next day and all three were fixed. Every number check passed, because they
  answer *"is this number free?"*. `git grep -il "self.append" bugs/` would have
  found bug-560 in one command. Withdrawn in `fdad98ccc`. **And when you DO file
  items recorded elsewhere, go back and edit the originating document** — the
  un-updated list is what produced the duplicates.
- **When a bug is already fixed, a RED is impossible — use a NEGATIVE CONTROL.**
  Re-adding only the `callback_referenced` arm restored `exit 139` on bug-589's
  verbatim program (5/5 runs) while main gave `c=n0` across 50. That is what
  distinguishes "fixed" from "never reproduced here".
- **Ancestry does not prove a binary contains a commit.** A build that STARTS
  before a peer's commit lands passes `git merge-base --is-ancestor` while
  emitting the old code — measured at 89 seconds of overlap. It presents as gate
  diffs that exactly match a recent commit's blast radius and nothing else.
  **That signature means suspect your binary, not your change**, and never
  "resolve" it by re-summing: that writes a stale compiler's hashes over goldens
  belonging to a landed change.
- **"No committed program reaches it" and "no valid program can reach it" are
  different claims**, and only the second licenses a deletion (bug-548).
- **To test a cache, find an input the cache cannot see.** Ed25519 is
  deterministic, so "the same bytes came back" is equally true of a full
  recompute — that assertion passed against unfixed code (bug-579).
- **An example is a `&'static str` the compiler never reads.** bug-559's examples
  were compiled and RUN before shipping. bug-472 (man examples are never
  compiled) is still the instrument gap behind every one of them.

## 2026-09-12 — repository security intake, worked

**All three HIGHs from the 2026-09-11 intake are addressed.** They were entirely
inside `repository/`, which makes them a different kind of work from everything
above: no IR, no goldens, **the artifact gate is structurally blind to all of
it**. The instrument is the `mfb_repository` unit suite and its loopback-HTTP
stub registry. Say so explicitly in any future repository bug — a green gate
there proves nothing at all.

**The repository security intake is CLOSED.** Nine bugs: eight landed, and 581
closed after Phase 1 (Phase 2 won't be done — owner decision 2026-09-12). `repository/` is
free for other work.

| Bug | Sev | Outcome |
|---|---|---|
| 578 | HIGH | **Landed** `f2368455b` — absolute MFPC section/pool/export/meta ceilings. |
| 581 | HIGH | **CLOSED** — route binding landed `ed87c111a`; Phase 2 won't be done (owner decision 2026-09-12: the registry is trusted by design). |
| 582 | HIGH | **Landed** `ac1a6ec79` — every log-pin advance is consistency-proof-gated. |
| 583 | MED | **Landed** `01d3529d1` — pairing approval is the CODE, not the relay-visible lookup. |
| 585 | MED | **Landed** `c7e7f1fec` — a redirect hostname is resolved before the hop is followed. |
| 586 | MED | **Landed** `9f1a0879d` — metadata DB + WAL/SHM are service-private. |
| 579 | MED | **Landed** `460b983dd` — memoised signed tree head, bounded anonymous log routes. |
| 584 | MED | **Landed** `a869dd247` + `9530d72a8` — one-time init, authenticated renewal, root-version floor. |
| 580 | LOW | **Closed** `e18cbb6fa` — **premise disproved**; separation made explicit (below). |

Also landed: `7ed3fa226` and `f9569fe70` (see below). Archived as already-fixed
and verified at HEAD: **549** (`8144872bd`), **551-inline-trap** (`1c83b7dda`).

**Merged-tree verification, not per-branch**: `cargo test -p mfb_repository
--no-fail-fast` = **356 lib + 21 bin, exit 0**, re-run from a clean
`git worktree` after 579 and 585 landed in parallel. Neither parent's green run
is evidence for the merge, and a clean worktree is the only place the
untracked-fixture class of defect shows.

### Findings that generalize

- **A signature over values the RESPONSE supplies binds nothing.** bug-581's
  name binding verified `name_binding_message(response.owner,
  response.ident_fingerprint)` — a binding of the response to *itself*. It looked
  like authentication for as long as nobody asked "authenticating *which*
  question?". When you see a signature check, find the request-derived value in
  the signed message. If there isn't one, it is decoration.
- **A safety gate placed in a CALLER is not a gate.** bug-276 R2 got
  verify-before-pin right but installed it in `verify_log_consistency`, one of
  two callers of the pin-advance path, leaving `fetch_checkpoint` free to skip
  it. Every later call site then had to re-derive the choice, and `pkg install
  --proof` got it wrong — reaching the log *only* through the unsafe helper. Fix
  is to make the unsafe spelling not exist, not to pick the safe caller again.
- **Two fields of one response can need two different comparison rules.**
  bug-581's `ident` is echoed verbatim by the server (compare exactly); its
  `owner` is returned as `owner_display` from a case-FOLDED lookup (compare
  folded). The obvious uniform fix — compare both exactly — would have refused
  every mixed-case `Alice#pkg`. Only measuring the server tells you which is
  which; this is the fifth time a guard here nearly shipped rejecting valid
  input, and the positive pin is what caught it again.
- **A pin whose subject is untracked is not a pin** (`7ed3fa226`). bug-578's
  sole "do the new ceilings refuse real packages?" test read
  `packages/libsnd/libsnd.mfp`, which `.gitignore:34` excludes. It passed only
  on a machine that had already built libsnd and failed on every fresh clone and
  in CI. Found by running the suite in a clean worktree, where it was the only
  failure out of 343. That is the fourth instance of the backlog's own
  "harness and code drift, harness reports success" shape — and the sharpest,
  because the check could not run *anywhere* but one laptop.
  **When a test reads a file, check `git ls-files` says the file is in the repo.**
- **Sizing a rate limit is a measurement, not a round number** (bug-579).
  `verify_publish_inclusion` makes three log requests and `pkg install --proof`
  calls it once PER DEPENDENCY, so a 200-dep install is a ~600-request burst
  from one IP — shared by everyone behind a NAT or CI egress. That number had
  just GROWN, because bug-582 added a consistency proof per pin advance. A
  budget picked from the attacker's side would have broken installs and looked
  correct in every test. Derive it from the client, and leave a test that forces
  the next person to.
- **To test a cache, find an input the cache cannot see** (bug-579). "The same
  bytes came back" proves nothing when Ed25519 signing is deterministic — a full
  recompute is byte-identical. The first draft of that test asserted exactly
  that and would have passed against the unfixed code. If you cannot name an
  input the cache is blind to, your test is not observing the cache.
- **A permissions fix in an image is undone by the volume it protects**
  (bug-586). `chmod 700 /data` in a Dockerfile is correct and nearly
  irrelevant — a mount replaces that inode. Any at-rest guarantee on mounted
  data must be re-established by the process at every start.
- **Tightening and widening are not symmetric** (bug-586). The obvious
  `set_permissions(0o600)` passes every negative test and silently relaxes an
  operator's `0400`. Assert the `0400` survives, or the fix is "set access to
  what I assumed" rather than "remove access".

### bug-580 was NOT a defect — and that is the interesting part

Its premise was false. Measured on the pre-fix store, **all five** role-colliding
creation paths already refused, with `UNIQUE constraint failed:
keys.fingerprint`: `keys.fingerprint` is declared `NOT NULL UNIQUE` **globally**
(`store.rs:464`), so no two rows anywhere may share a public key. The report
reasoned from `register_owner`'s body — which indeed never compares the two keys
— without checking the schema the insert lands in.

Two things make it worth the time anyway:

- **A security property can be true by accident, and an accident is not a
  guarantee.** Separation held because of an index that exists for a different
  reason. Worse, this bug's *own non-goals* ask for that index to be loosened
  ("do not prohibit two different accounts from independently choosing the same
  public key" — which the global UNIQUE currently DOES prohibit, measured).
  Whoever loosens it would delete role separation as a side effect with nothing
  failing. The account-scoped check now stands on its own so the two properties
  can move independently.
- **The report's blast radius was wrong in the direction that matters.** It named
  two insertion paths; there are five, and the two it missed include
  `issue_publish_token` — an `auth`-role key for a DELEGATED, EXPORTABLE
  credential handed to CI. A per-site fix would have been written from that same
  wrong list, so every `keys` insert now goes through one writer with a
  `key_insertion_has_exactly_one_writer` census guarding it.

Still open deliberately: two accounts cannot share a public key, diverging from
the stated non-goal. Allowing one key to authenticate as two accounts is a policy
decision, not a drive-by edit; the current behaviour is now asserted so a change
is deliberate.

### bug-581 Phase 2 — CLOSED as won't fix (owner decision, 2026-09-12)

The owner ruled: "this is an issue but not a fixable issue. you have to trust the registry at
some point, or dont use it." The analysis below is kept as the record of the residual risk.

Phase 1 closed *substitution*. *Staleness/truncation* (a correctly-identified
version list with a newer version omitted) is not closed, and cannot be with the
data on hand: `snapshot.indexHash` commits to the **global** index, there is no
route that serves the full index, so a client holding one package's response
can never recompute it. Closing it needs a per-package commitment signed by the
**offline** snapshot key — either per-package targets in `snapshot.json` or a
Merkle root plus inclusion proof reusing `log.rs`. Both are wire/metadata format
changes, and both force a ruling that is not technical: **what does a client do
when `snapshot.json` carries no per-package commitment?** Fail closed breaks
every deployed registry; fail open means the fix does nothing. Full analysis in
the bug doc.

## 2026-09-12 — plan-125 documentation-review intake

Filed, not fixed: plan-125 is documentation-only by user instruction.

- ~~**603 LOW**~~ — **LANDED `333b23197`** (2026-09-13; archived to
  `bugs/completed/`). `color::hsl`/`hsla`/`rotateHue` raised `ErrOverflow` for a finite
  hue past ~3.3e21 degrees: `__color_wrapHue` took the fractional turn through
  `Integer`-returning `math::floor`. Now `hue MOD 360.0`; 40 `.ir` goldens and 9
  `.ncodesum`s regenerated, each proven to be only the helper.

## 2026-09-11 — repository security review intake

- ~~**578 HIGH**~~ — **LANDED `f2368455b`.** A bounded repository upload could
  force multi-hundred-megabyte allocations through unbounded MFPC
  string/section/export counts. Measured pre-fix: 48 MiB of body -> 12,582,912
  `String`s -> **288 MiB of headers alone**. Note bug-276 R8's existing
  `count.min(bytes.len() / 4)` cap did NOT help — `bytes.len() / 4` is exactly
  the number of empty entries the attacker supplies. Only an absolute ceiling
  closes it; a relative one is satisfied by the attack payload.
- **579 MEDIUM** — anonymous transparency-log routes rebuild and materialize
  the full log without limits or caching.
- **580 LOW** — registration/linking allow an auth key to equal the ident key,
  collapsing the intended credential boundary.

## 2026-09-11 — repository protocol-audit intake

- ~~**581 HIGH**~~ — **CLOSED, `ed87c111a`.** `fetch_index` accepted an index
  not bound to its requested ident. Route binding landed. Binding to signed snapshot
  metadata (Phase 2) won't be done: the owner ruled on 2026-09-12 that the registry is
  trusted by design.
- ~~**582 HIGH**~~ — **LANDED `ac1a6ec79`.** Larger signed transparency-log
  forks overwrote a client pin without a consistency proof, and publish
  inclusion used that unsafe path — which made it reachable from `pkg install
  --proof`, whose ONLY log contact was that helper.
- **583 MEDIUM** — a relay-visible pairing lookup can enroll an attacker auth
  key, despite the code remaining secret.
- **584 MEDIUM** — rerunning root initialization replaces the root anchor with
  no authenticated old-to-new transition.

## 2026-09-11 — repository file-by-file security-review intake

- **585 MEDIUM** — a redirect hostname is allowed without checking whether DNS
  resolves it to a private or link-local address, retaining a client-side SSRF
  path for blob requests.
- **586 MEDIUM** — the shipped container leaves its SQLite database and
  key-bearing sidecars at default permissions, exposing server and metadata
  signing credentials to another local UID.

## ⚠️ THREE BUG NUMBERS COLLIDE — 550, 551, 552

A peer session filed its own 548–552 concurrently with this one. **550, 551 and
552 each name two completely different bugs.** As of 2026-09-12, **all six are fixed** and
archived under `bugs/completed/`:

| # | first bug | second bug |
|---|---|---|
| 550 | every `debug_assert!` is decorative — CI builds release (`157a8dc52`) | `collections::append([], x)` type-checks then fails to build (`2203554bb`) |
| 551 | an `EXPORT LET` package constant has no type for an importer (`9a5aacb6e`) | an inline `TRAP` on a `RES … STATE` write panics the compiler (`1c83b7dda`) |
| 552 | the riscv64 linker scans relocations quadratically (`aea1216bf`) | the three Level-2 global optimizer rows cannot fire (`d83714554`) |

**Never renumbered.** Renumbering a peer's in-flight documents in the shared checkout
silently destroys someone's work, so the collision was recorded rather than fixed, and
both documents of each pair kept their number. **A reference to "bug-550", "bug-551" or
"bug-552" is still ambiguous** — cite the title or the archived file name too.

The underlying cause is the one already recorded for rule codes and plan numbers:
`ls bugs/` under-reports, because a number claimed on an unlanded branch is
invisible. `git log --all --grep=bug-NNN` catches most of it and did not catch
this, because the peer had not committed when I picked.

## 2026-09-06/07 — what landed

**27 bugs archived** (`git log --since=2026-09-06 --diff-filter=R --name-status`):
456, 479, 487, 488, 515, 516, 527, 543, 551, 552, 553, 554, 555, 556, 558, 560,
561, 562, 565, 566, 567, 568, 569, 571, 572, 573, 574.

The bulk is one connected cluster: **thirteen scope-drop / ownership leaks**
(536-B2, 560, 561, 562, 565, 566, 567, 568, 569, 571, 572, 573, 574), which
`tests/runtime/rt_scope_drop_leaks.rs` now holds together — **87 cases, green as a
set**. Two of them were crashes rather than leaks (562's callback SIGSEGV, and
572's `http::route` use-after-free caught before it shipped).

### What this cluster taught, worth applying to the next one

- **A runtime pointer-identity guard beats a whole-program proof.** Six of the
  fixes free only after comparing against the value they do not own, so soundness
  is local. Where the two pointers are provably the same stack slot, skip the
  compare rather than emit one that could go the wrong way.
- **Enumerate, and assert the enumeration is TOTAL.** 567 replaced a predicate
  with a wildcard-free `match` (a new `NirOp` is now a build error); 566 and 572
  assert set equality over a registry. 572's first gate would have shipped a
  use-after-free precisely because it had a default.
- **Pin the negative side by equality or asserted growth, never flatness** —
  flatness cannot distinguish "correctly declined" from "wrongly freed".
- **Golden attribution is per FUNCTION**, not a count: "N changed, every one
  gained ≥1 free, none gained an allocation, `dataObjects` byte-identical".
- **The docs' stated causes were wrong about as often as they were right** — 560
  (frees the wrong SIZE), 568 (the callee's `RETURN` shape, not the payload type),
  566 ("flat outside a TRAP" — false), 574 (`emit_raw_call` copies nothing), 573
  ("cannot be helper-local" — it can). Measure the doc's own contrast first.

## Working rules for this pass

- **Model:** fable for CRITICAL, opus for everything else. There is currently
  **no open CRITICAL**, so every dispatch below is opus.
- **Concurrency:** two background agents, plus the lead working a third bug
  directly.
- **Concurrency has a COMMIT hazard the gate lock does not cover.** Agents leave
  uncommitted edits in the shared checkout, so `git add <file>` on a file an
  agent is also editing sweeps their work into your commit. Before committing,
  `git status --short` and confirm every file you stage is one only you touched;
  if a peer is in the same file, pick a different bug rather than racing. Pick
  bugs whose blast radii are *file-disjoint*, not merely topic-disjoint — this
  pass had to defer bug-586 for exactly this reason (`store.rs`).
- **Landing:** commit and merge each bug as it completes — never batch.
- **Memory bugs carry an extra gate.** For any bug touching allocation,
  aliasing, ownership or drop (`487`, `536`, `538`, `479`, and the landed
  `495/496/497/498`), it is not enough that the tests pass: the fix must be shown
  **not to change the memory semantics of the language**. Required evidence, per
  the pattern established on bug-496/497:
  1. the RED test flips green;
  2. a *correct* program's observable behaviour is unchanged — name the
     documented contract in `.ai/collections.md` / `mfb spec` §14 the fix now
     realizes, and show the fix only ADDS a check/copy/lock rather than altering
     a value's lifetime or identity;
  3. the artifact gate's golden delta is confined to the emitting fixtures, with
     everything else byte-identical (that containment IS the semantics proof);
  4. a positive pin, not just the negative one — e.g. bug-497's
     `every_byte_list_producer_still_passes_the_write_header_check`, which
     caught that a new guard could reject valid programs.

## Tier 1 — audit-3: DONE

Nothing open. 499 (spawned child inherits fds), 504 (emitted PE has no ASLR) and
510 (text-decoder DoS cluster) are landed and archived, as are 514, 535, 538,
539 and 545. **Do not re-dispatch these** — the table that used to sit here said
"agent running" for all three and was stale for a full session.

## Tier 2 — HIGH: one open, blocked on planned work

*(Updated 2026-09-12. This heading used to read "there is NO open HIGH", which stopped
being true when 601 was filed.)*

- **601** — a `MUT` copy of a non-flat list aliases its source, so an in-place `append`
  on the copy segfaults and a short variant silently computes a wrong value. The
  pointer-`String` records half (the crash) is fixed by **plan-132**; the recursive-type row
  (`List OF Tree`, a wrong value) remains and rides on bug-536 shape C.
- ~~**581**~~ — **CLOSED 2026-09-12.** Phase 1 landed (`ed87c111a`); Phase 2 won't be done
  (the registry is trusted by design).

~~**536**~~ — **CLOSED 2026-09-13.** Shapes A (`f9be6e128`), B (`cd8699103`) and B-2
(`b845db0de`) are fixed; shape C is plan-134 (A–H), not a bug. Archived to
`bugs/completed/`.

### A correction, because this section was wrong twice in one day

It first said "536 is the only open HIGH" with shape B-2 listed as remaining —
stale by six days. Correcting that, I then made it worse: I filed three "new"
HIGHs (587/588/589) out of bug-536's list of defects "recorded rather than filed
so the numbering does not race with a peer session", and promoted them here as
"the real HIGHs".

**All three were duplicates of bugs that were already fixed.** That note in
bug-536 was stale: the defects were filed the next day as **560, 561 and 562**
(`8c57683f3`), and all three were fixed on 2026-09-07. 587/588/589 are withdrawn
(`fdad98ccc`) and moved to `bugs/completed/`.

**The lesson is about which question gets asked.** The number checks
(`ls bugs/ bugs/completed/`, `git log --all --grep=bug-NNN`) all passed, because
they answer *"is this NUMBER free?"* — it was. Nobody asked *"is this DEFECT
already tracked?"*. `git grep -il "self.append" bugs/` would have found bug-560
in one command.

**Two durable rules:**
- **Before filing, search for the DEFECT, not the number** — grep
  `bugs/completed/` for the symptom, the function name and the idiom.
- **A "recorded but not filed" note is a dated claim about the past.** Check
  whether it is still true before acting on it, and when you DO file such items,
  go back and edit the originating document — the un-updated list is what
  produced the duplicate six days later.

The proximate enabler was also a stale doc: `.ai/codegen-invariants.md` still
described `function_returns_fresh_string` as excluding callback-referenced
functions and said "dropping the arm is the fix; it wants its own callback-ABI
audit" — five days after the arm was dropped. Corrected in `7faff2d3b`, with the
live guards named inline so it cannot be silently re-added.

That fix was additionally proven load-bearing by **negative control**: re-adding
only the `callback_referenced` arm restores `exit 139` on the filed program (5/5
runs), while current main prints `c=n0` and exits 0 across 50 runs. Worth copying
as a technique — when a bug is already fixed, a RED is impossible, and the
negative control is what distinguishes "fixed" from "never reproduced here".

### bug-536 itself — closed out except for a design question

Three of its four parts are done:

- **Shape A** — `RETURN <constructor>` abandoned the fresh block. Fixed
  `f9be6e128`, merged `c210cc67d`.
- **Shape B, native half** — an unbound `String` from a *native* producer was
  never freed (`acc = acc + len(toString(i))` leaked 64 B per evaluation). Fixed
  `cd8699103` by **fail-closed freshness provenance**: a producer that just
  `arena_alloc`ed the block it returns marks it, and `register_pending_temp`
  frees a bare `String` only on that mark. Unmarked keeps leaking, never wild-frees.
  Golden delta was 142 `.ncodesum` + 4 `.ncode` + 1 `.mir` and **zero**
  `.run`/`build.log`.
- **Shape B-2, callee half** — a `String` returned by a user / `.mfb`-bodied
  function. **FIXED `b845db0de`** (2026-09-06). It was what had cost the decoders: `csv::parse` is
  byte-identically unchanged by the native fix. The bug doc's old claim that "csv
  has a SECOND leak that is NOT shape B" is **wrong** and now corrected there — it
  is shape B one level up (`row = append(row, __csv_fieldValue(...))`). Needs a
  transitive `function_returns_fresh_string` NIR predicate; it is a
  **double-free** risk, not a leak risk, so it wants its own change and audit.
- **Shape C** — a value of a recursive type is never freed. **Blocked**: it needs
  recursive COPY-insertion, which does not exist, and the naive fix is a double
  free. It is a design pass, not a bug fix. Do not dispatch it as one.

514 is landed. 519, 532 and 535 are landed; 538 is landed and 539 also fixed a
pre-existing GTK draw-callback SIGSEGV and put the Linux GTK app backend under
byte-identity coverage for the first time. 532 **unblocked bug-534's `split`**.

## Tier 3 — MEDIUM, grouped so a single agent can take a cluster

**Regex/strings surface: the whole cluster is LANDED.** 529/531/533
(`2860dd7e7`, `5e93d26a3`, `426660224`), then 530 (`f75616ed2`), 528
(`f071d0f45`) and 534 (`fe7903170`).

**Three findings from it outlive the bugs:**

- **A THIRD dead-handler miscompile**, and a different shape from the first two.
  `strings::left`/`right`/`padLeft`/`padRight` raised `ErrInvalidArgument` while
  declaring `errors: vec![]`, so `inline_builtin_is_infallible` proved them
  infallible, the compiler warned `TYPE_INLINE_TRAP_DEAD_HANDLER`, **deleted the
  live handler**, and the program aborted with `7-705-0002`. Fixed `6d9f4b79a`.
  bug-486 and bug-533 were name-keyed-over-an-overload; this one is simply a
  member lying in its descriptor. **A function-level `TRAP` test cannot see any
  of the three — write the INLINE form.**
- **The invariant that should have caught it never runs** — filed as **bug-550**
  (the `debug_assert!` one; the number collides, see above).
  `raise_error_bare`'s declaration check is a `debug_assert!`, and CI builds
  release on every job, so all **55** `debug_assert!`s in the tree are compiled
  out everywhere. **Landed `157a8dc52`** (2026-09-06).
- **Two branches editing one package both shift its embedded line numbers**, so
  neither parent's `.ir` goldens are right for the merged tree. 528 and 534
  collided on exactly that; resolving the conflict by picking a side would have
  produced goldens matching neither compiler. Regenerate post-merge — the gate
  named the two affected fixtures precisely (2 of 1932).

**Resource / close contracts** (one agent): the cluster is **complete** — 524,
525, 526, 522 and 523 are all landed.
**522 is landed (`09453380a`)** — the `transfer` page's list was stale since
bug-464; `transfer`, `accept` and the intro now agree with the registry and all
three name the five resources that may not cross. A pin asserts every `sendable`
bit against an explicit table.
**523 is landed (`593d68965`)** — each `types` page states the record-field and
collection-element shapes once and derives transferability from the `sendable`
bit (a new `unsendable_reason` carries the per-resource reason); `mfb man
variable` gained a runnable example of each shape. It also corrected two FALSE
statements — `process`'s package description and `mfb spec stdlib transports`
both said a handle may not be a record field, which §15.4 and
`record-res-field-export-rt` refute.

Follow-ups these left behind, both already-filed bugs rather than new ones:
`scripts/man-run-examples.sh` reaches package pages only, so `mfb man variable`'s
new examples were verified by hand (**bug-472**); and `audio::close`'s new
raise-on-double-close has no runtime proof on any host for want of a device.
**524 is landed (`be88539c8`)** — `process::close` is now `process::closeInput`;
the behaviour did not move and zero `.run` goldens changed. It also leaves 523 a
concrete correction: `process`'s package description claims a handle "cannot be a
field of a record", which is false.
**525 is landed (`9053e5eb0`)** — every built-in `close` now refuses an
already-closed handle with `ErrResourceClosed`, and both `listen` members default
`backlog` to `128`. It was never a decision waiting to be made: `mfb spec language
resource-management` §15 already required the raise, so `tls` and `audio` were the
non-conforming side and `src/docs/spec/stdlib/17_transports.md` was the stdlib spec
contradicting the language spec. Proven before/after on all three TLS backends
(macOS Network.framework, box 2228 OpenSSL, box 2230 Schannel); `audio` has no
runtime proof anywhere (no device) and rides a lowering pin.
**526 is landed (`521b731e0`)** — `mfb man tls poll` printed a signature that did
not compile. A registry-wide renderer pin now fails any rendered signature whose
collection element is an unmarked resource; it found exactly one violation, so
the sibling census is complete.

**App backends**: **541 is landed (`0ba90b19f`)** — all three gates, with a real
runtime RED on box 2230 (`gate01=size80` -> `gate01=raised` against the console
oracle). **540 is PARTIAL**: WIN-01 and WIN-05 landed (`a02bd12df`), and WIN-04 landed
in round 3 (`6ca7e8b8e`). The Windows smoke run on box 2230 passed on 2026-09-12. Only
WIN-02/03 remain. WIN-05 was a NEW find while fixing 541 —
Windows `term::on` reset 3 fields where every other backend resets 7.

**WIN-02/03 are blocked on an INSTRUMENT, not on work.** Headless Windows never
reaches `WM_SIZE`, so proving a resize needs a `term::` twin of
`MFB_CANVAS_RESIZE_W/_H`. That adds test-only surface to the product, so it is an
Open Decision. WIN-04 was fixed by sharing `emit_app_io_write`'s cluster walk, landed
in round 3 (`6ca7e8b8e`).

**A THIRD bug was found here and it is the one worth remembering.**
`scripts/test-winapp.sh` named `canvas::rgb`, which plan-122-D's canvas migration
removed the same day. Under `set -e` that is a hard stop at the BUILD, so 17
assertions — the canvas frame, the entire Vulkan section, both resize runs —
exited "passing" without executing. This is the ONLY instrument in the repo that
runs a Windows binary at all, so its silence removed the sole runtime coverage
for a whole platform. Verified independently: `canvas::rgb` does not exist.

That makes three findings this session of the same shape — **a harness and the
code it exercises drift, and the harness reports success**: bug-470's guards
covered 3 of 11 writers; `curve448_secret_paths_are_branch_free` covered one of
two fields; this script covered one of two halves of itself. None FAILED. Each
passed while checking less. When a rename lands, grep `scripts/` for the old
spelling — nothing else will tell you.

**datetime**: 520 — **re-scoped 2026-09-13** by owner ruling: `datetime` must be correct on
its own, without a zone database. A standalone audit found host-zone resolution correct
(macOS + box 2223 vs Python `zoneinfo`) but ten defects remain, so 520 stays open (large):
S1 the offset writer drops seconds (`-04:56:02` → `-04:56`, reachable from `toLocal`);
S2–S5 `parseIso`/`parse` accept an offset's `:SS`, trailing text, `+05:75`, and raise the
wrong code for `+24:00`; S6 `addDays(dt, 0)` moves a repeated hour; S7–S9 page claims
("pure", missing error lists, the `fixedOffset` formula); S10 no fixture sets `TZ`.
Owner decisions: S1 **write seconds**; S6 **keep the offset** when it is still valid.
The file is now `bugs/bug-520-datetime-is-not-correct-standalone.md`. A datetime
**bug-603** was filed the same day for S1–S3 and merged into 520, with no file of its own.
That number was already taken: the colour hue-wrap `bugs/completed/bug-603-color-hue-wrap-overflows.md`
(plan-125 intake above) is a different bug.
**Named zones moved out of 520** into **plan-135-A–D** (`packages/timezones`, a source
package over vendored IANA tzdb 2026d, never the host's zone data; zone names match
ignoring case). plan-135-D cannot start until 520 closes. 518, 519 and 521 are landed.

**crypto**: the cluster is **complete**. **515 is landed (`b399e3363`)**, as are
**511 (`e24914d5d`)** and **517 (`5c2024f71`)**.

**517 was the one backlog item blocked on a ruling, and the owner gave it**: the
SHA-1 advisory is scoped to the USE — `hash` warns, `hmac`/`hkdf`/`pbkdf2` do
not. That reverses `plan-109-A:92` ("regardless of which public function consumes
the selector"), so the plan is left unedited as a record of what was decided
then and the reversal lives in the bug doc and in
`ir::verify::values::hash_selector_use_is_sound`.

Two things from it that generalize:

- **Suppression is fail-closed, and that was a choice inside the ruling.** The
  ruling names members that should not warn; implementing it as "fire only at
  `hash`" would make silence the default. Implemented the other way round, so a
  local-bound selector, a `MATCH` literal, a bare occurrence and a value nested
  inside the argument all still warn.
- **A builtin member has TWO call-target spellings in IR** and only one is
  obvious. `crypto::hash` arrives dotted (`crypto.hash`); the `.mfb`-bodied
  `hmac`/`hkdf`/`pbkdf2` arrive as `#crypto_hmac`, the `internal_name::internalize`
  form of the package's own `__crypto_hmac`. Matching only the dotted name
  suppressed NOTHING and looked correct. Any future predicate keyed on a call
  target must accept both, via the mangling contract.

**511 left a follow-up worth doing** (a test-coverage gap, not a defect). The
report named ONE secret-dependent branch; there were two —
`__crypto_pack25519` also branched on the borrow out of its trial subtraction,
and one of the values packed there is the X25519 shared secret. It survived
because `curve448_secret_paths_are_branch_free` enforced the property **for the
448 field only**, while the constant-time primitive and its packer already
existed. Same "two lists" shape as bug-470 and bug-533. The durable fix is a test
that ENUMERATES the curve fields and asserts a branch-free secret path for each,
so adding a curve fails until it is covered.

**registry / supply chain** (audit-3 MEDIUM carryover): **all landed**:
489 (response terminal injection, `a1cd9d0c7`) · 490 (client redirect credential leak,
`18f589667`) · 491 (`pkg install` not bound to the lock, `0f5256342`)

**Older carryover**: **453, 454 and 483 are landed** (`f332f18e6`, `94b2ec1e1`,
`7b0ab81be`). The rest of this list has landed too:
- 479 (inline TRAP on thread start): `694dee2b7`
- 487 (state-mutating operand UAF): `56b368996`
- 527 (range parameter naming): `177cdd2e7`
- 515 (memory-hard password KDF): `b399e3363`
- 472 (man examples never compiled): `17c424988`
- 543 (spawn fd parity): `e36ebcedf`
- 488: CLOSED after its clean period (`e5f705c13`)

**Still open:** only 484 (`picture::drawItem` never renders — x-large, sequenced AFTER
plan-116-I) and 520 (named zones, huge — re-scoped 2026-09-13, see **datetime**).

**Filed that day, all found while fixing something else — none is a regression — and
all since landed:** 550 (55 `debug_assert!`s that never run, because CI builds release;
`157a8dc52`) · 552 (riscv64 linker quadratic, unreachable until 453 removed the ceiling
above it; `aea1216bf`) · 553 (28 `tls`/`tcp` members declare `errors: vec![]`;
`0fbccd28d`). 550 and 552 are the collided numbers; see the collision table above.

**The lesson the cross-platform cluster paid for: name the instrument.** All
three needed something the artifact gate structurally is not.
- **453** — the gate reported 1930/0, *identical to the untouched baseline*, and
  that zero IS the containment proof, because relaxation is a no-op in range. It
  says nothing about execution; only **box 2229** (real riscv64) shows a relaxed
  five-rung chain runs.
- **454** — proved by a **negative control** on box 2230: with the separator left
  POSIX, cross-build and gate stay GREEN while the program fails on Windows.
  That demonstrates the instrument gap instead of asserting it, and is the
  cheapest way to prove a per-platform fix is the fix.
- **483** — the Windows row of its matrix had only ever been READ from source;
  measuring it changed the answer. Its own doc's proposed macOS design turned out
  to be a use-after-free, found by running it, not by reading it.

**And the gate lock (bug-470) refused one of MY runs**, correctly: a subagent's
`test-accept.sh` held the tree lock, my gate exited 98 having checked nothing,
and `DIFFS=0` next to a refusal would have read as success if refusal shared exit
code 1 with "found diffs". The distinct code is what made it detectable. The same
collision hit the agent minutes earlier as 4 phantom mismatches in fixtures the
bug does not touch — same cause, one unmistakable outcome and one plausible wrong
one.

**Resource bookkeeping holes found by bug-535's sweep** (both hidden by the same
"any other call into the package" condition, both reproduce on `4d56f1a1a`):
545 and 546 are both landed. **546 (`6da957747`) is worth reading before any
codegen work that classifies a type**, because its root cause generalizes: every
`codegen::builtins::is_resource_type` / `is_thread_sendable_resource_type` call
answers for the BUILT-IN registry only, and a user-declared `RESOURCE` fell
through to a default of `true` for both flatness modes — "this handle is a flat
copyable block that may be relocated into another thread's arena". Use the
model-aware `is_resource_nominal` / `is_sendable_resource_nominal` instead. The
same blind spot had a SECOND consumer (`defer_resource_flag`), which meant
bug-425's guarantee never held for user resources. **479 shares 546's error
message**, so read them together (479 is also landed, `694dee2b7`). The invariant is
recorded in `.ai/resources-packages.md`.

## Tier 4 — test-infrastructure flakes (cheap, and they are costing us now)

**Nothing open.** Every row that used to sit here has landed:

| Bug | Sev | Landed | Title |
|---|---|---|---|
| 488 | LOW | CLOSED `e5f705c13` | `rt_tls_connect_allow_self_signed` port gate is per-process |
| 456 | LOW | `854c99fdd` | `mfb opt` sweep level-variant ncode goldens |
| 472 | MED | `17c424988` | man examples are never compiled |

537 is landed. 488 and 537
produced false reds on four separate suite runs during the audit-3 fix pass,
every time two `cargo test` runs shared the machine — which is exactly the
agent-plus-lead setup this backlog prescribes.

**470 is landed (`fea98e3cb`)** — and it turned out to be three fixes, not one.
The prior branch's per-tree `mkdir` lock was the right mechanism; what was
missing was everything around it. Both halves of the defect were REPRODUCED
(the doc had said "inferred, not reproduced"): pre-fix, an `artifact-gate` ran
to completion in the same tree as a live `test-accept`, and a `test-accept` in
one worktree refused one in another. The fix itself leaked the lock on
`test-accept.sh`'s SUCCESS path (`trap` replaces, it does not chain — only
INT/TERM survived, so a killed run released correctly and only success leaked).
And the lock covered 3 of the **11** scripts that rewrite fixture dumps in-tree;
regenerate-then-gate is a normal workflow, so the `regen-*` scripts mattered.
The transferable lesson is in `tests/gate_lock_covers_every_writer.rs`: a
recogniser for "which scripts contend" was written three times and
under-reported every time, so it is now an exhaustive classification with a
blindness guard. 488, 456 and 472 have since landed too (see the table above).
