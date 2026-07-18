# plan-48-B: client vendor-blob upload, download, and verification

Last updated: 2026-07-16
Effort: medium (2h–4h)
Depends on: plan-48-A (the `PUT`/`HEAD /blob/<hash>` endpoints), plan-46-B
(section 10 exists and carries the vendor hashes)

The client half of vendor-library distribution. `mfb pkg publish` uploads each
`vendor` locator's file as its own blob before publishing the `.mfp`; `mfb pkg
add` / `mfb pkg install` download every vendor blob the package's section-10
table names and **verify each one's sha256 against the signed table** before it
is allowed to exist under a usable name.

This is what closes plan-46-C §1.1 — the honest admission that a vendor locator
resolves and verifies but cannot load, because nothing put the bytes anywhere.

The single behavioral outcome: `mfb pkg add acme#sqlite3` on a machine that has
never seen the library leaves a verified vendored `.so` on disk, and plan-46-C's
build-time verify finds it without the user having placed a single file by hand.

References (read first):

- `repository/src/client.rs` — `fetch_blob` (`:958-985`, the hash re-check at
  `:981` is the entire security argument), `post_json` (`:1038-1051`),
  `package_request` (`:1026-1036`), `Client::new()` per request (`:961`, `:1045`,
  `:1056`), `ensure_transport_security` (`:29-54`).
- `src/cli/pkg.rs` — `publish_package_project` (`:121-218`), `pkg add` registry
  path (`:513-582`), `pkg add file://` path (`:458-505`).
- `src/cli/resolve.rs` — `pkg install` (`:89-145`), the serial `fetch_blob` loop
  (`:122-143`), `LockedPackage` (`:37`), `LOCKFILE_VERSION` (`:29`).
- `src/cli/mod.rs` — `install_verified_package` (`:63-81`) and the
  stage-verify-rename discipline (`:21-49`, `create_new(true)`, bug-27).
- `src/cli/build.rs` — `classify_installed_package` (`:1143-1232`), the plan-23
  §3.5 chain; the `Unsigned` short-circuit at `:1160-1162`.
- `planning/plan-46-C-consumer-resolution-load.md` §4.4 — the verify this plan
  feeds, and §1.1 — the gap it closes.

## 1. Goal

- `mfb pkg publish` uploads every `vendor` locator's file as its own blob
  (skipping any the registry already has), **then** publishes the `.mfp`.
- `mfb pkg add` / `mfb pkg install` download every vendor blob named by section
  10 and verify each against the table's sha256; a mismatch or a missing blob is
  fatal and leaves nothing usable on disk.
- Downloaded vendor files land where plan-46-C's build-time verify can find them
  without the user placing anything (§4.3).
- Transport can actually carry a 40 MB file: the 30-second whole-request timeout
  that would kill it today is gone (§4.1).

### Non-goals (explicit constraints)

- No server or protocol change (plan-48-A).
- No `mfb.lock` change. §3.2 explains why none is needed — the `.mfp` hash
  already pins section 10, which already pins every vendor hash.
- No global/shared blob cache. The client has no cross-project package cache
  today (§2.3) and this plan does not invent one; it is called out as follow-on
  work (§Open Decisions).
- No retry/resume. There is none anywhere today (§2.2); the timeout fix (§4.1) is
  the minimum needed for correctness, and retry is a separate concern that should
  cover every fetch, not just blobs.
- Do not weaken the bug-27 stage-verify-rename discipline (§4.4).

## 2. Current State

### 2.1 The download path is already the right shape

`fetch_blob` (`client.rs:958-985`) is hash-addressed, unauthenticated,
redirect-following, and self-verifying:

```rust
let url = format!("{}/blob/{}", repo_url.trim_end_matches('/'), hash);
let response = Client::new().get(&url).send()          // :961-963
let bytes = response.bytes()...to_vec();                // :977-980
if hex::encode(crypto::sha256(&bytes)) != hash {        // :981
    return Err("downloaded blob does not match the requested content hash");
}
```

It follows the S3 `302` implicitly via reqwest's default redirect policy
(`Policy::limited(10)`), which `repository/Cargo.toml:31-34` documents as
intentional. Being unauthenticated is safe precisely because of `:981` — the hash
check is the whole argument, and **it holds for any content**, which is why this
function generalizes to vendor blobs almost for free.

### 2.2 Three transport facts that block large blobs

- **A 30-second whole-request timeout.** reqwest blocking defaults to
  `Timeout(Some(Duration::from_secs(30)))`, and **no `Client::builder()` call
  exists anywhere in the tree**, so nothing overrides it. A blob that takes more
  than 30 s to transfer fails outright. For a `.mfp` of IR this never mattered;
  for a 40 MB shared library on an ordinary connection it is the common case, not
  the edge case.
- **`Client::new()` per request** (`client.rs:961`, `:1045`, `:1056`). Each
  blocking client spins up its own tokio runtime on a background thread. Fetching
  N blobs means N runtimes, N TLS handshakes, and zero connection reuse.
  Downloads are strictly serial (`resolve.rs:122`).
- **No retry, no resume, no `Range`.** One `.send()`; the first error is final
  (`client.rs:964`). The `.part` staging file (`mod.rs:33`) is a symlink-safety
  device from bug-27, not a resume buffer.

The client has also **never issued a PUT** and **never streamed a body** — every
upload to date is an in-memory base64 JSON string (`client.rs:1030`).

### 2.3 Install layout, and the absence of a cache

Packages are per-project; there is no shared store:

```
<project>/
  project.json          # deps + the pinned identKey per dep
  mfb.lock              # name/ident/requested/selected/hash/identKey
  packages/
    <name>.mfp                        # the installed package, one file
    .<name>.mfp.<pid>.<nanos>.part    # transient staging (mod.rs:33-36)
    <name>/project.json               # alt: source-form dependency (pkg.rs:1313)
```

Every `fetch_blob` is a fresh download — nothing is cached between projects or
between `pkg install` runs. `install_verified_package` (`mod.rs:63-81`) is
single-file by construction: stage one blob under an exclusively-created `.part`
name (`create_new(true)`, defeating symlink pre-planting — bug-27), classify it,
and `fs::rename` onto `packages/<name>.mfp` **only** if `Verified`.

`~/.mfb/<sha256(repo_url)>/` exists as a repo-scoped, private-perms directory
(`local.rs`) holding pinned keys, checkpoints, and sessions — but **no package
content**. It is the natural home for a shared blob cache; there is no precedent
to follow.

### 2.4 Diagnostic codes

Package-trust diagnostics live in `6-605-0001..0009`
(`src/rules/table.rs:1016-1069`, ending `REGISTRY_LOG_ROLLBACK`).
**`6-605-0010` is the next free.**

## 3. Design Overview

Three pieces:

1. **Transport hardening** (§4.1) — a shared, correctly-configured
   `reqwest::Client`. A prerequisite, not a nicety: without it a 40 MB blob
   cannot be fetched at all.
2. **Publish** (§4.2) — read section 10 from the just-built `.mfp`, `HEAD` each
   vendor hash, `PUT` the ones the registry lacks, then the existing
   validate→publish pair.
3. **Install** (§4.3-4.4) — after the existing `.mfp` verification succeeds, read
   section 10, fetch each vendor blob, verify each hash, and place it atomically.

### 3.1 The order is the correctness argument

Blobs go up **before** the `.mfp`, and come down **after** it. Both directions
follow from the same fact: the `.mfp` is the only thing that names the blobs.

- **Publish:** blobs first means a successful publish never leaves a section-10
  hash dangling (plan-48-A §4.4 enforces the converse server-side). A failure
  after blobs but before publish leaves unreferenced blobs — garbage, not a
  broken package.
- **Install:** the `.mfp` first, and **fully verified**, means section 10 is
  trusted *before* any hash in it is used. Fetching blobs first would mean acting
  on attacker-supplied hashes.

### 3.2 Why `mfb.lock` needs no vendor-blob list

Tempting, and unnecessary. `mfb.lock` pins the `.mfp`'s content hash
(`resolve.rs:37`). That hash covers the whole artifact, which covers
`packageBinaryHash`, which covers the payload, which contains section 10, which
contains every vendor hash (plan-48-A §3.2). **The vendor blobs are already
transitively pinned by the lockfile entry that exists today.** Adding them would
duplicate data that cannot disagree without the `.mfp` hash already mismatching.

(Worth noting while nearby: `LOCKFILE_VERSION` is `1` and is not currently checked
on read (`resolve.rs:29`). Out of scope, but someone should look.)

### 3.3 Unsigned packages get no vendor guarantee

`classify_installed_package` short-circuits to `Unsigned` before any verification
(`build.rs:1160-1162`). An unsigned package's section 10 is attacker-controlled,
so its vendor hashes authenticate nothing — downloading a blob that matches them
proves only that the blob matches what the file claims.

This plan does **not** invent a trust story for unsigned packages. It fetches
their blobs (they are still functional) but must not report them as verified;
`PACKAGE_UNSIGNED_REMOTE` (`6-605-0007`) already covers the surrounding case and
should be the single warning the user sees, not a second reassuring
"vendor blob verified" line beside it.

## 4. Detailed Design

### 4.1 Transport hardening (prerequisite)

**Status: implemented.** Landed ahead of the rest of this plan because it fixes a
latent bug for large `.mfp`s today, independent of vendor blobs.

Build **one** `reqwest::blocking::Client` and reuse it, replacing the
`Client::new()`-per-request pattern (`client.rs:961`, `:1045`, `:1056`).
Connection reuse follows for free, which matters immediately: a 7-slot binding is
7 sequential fetches that currently build 7 tokio runtimes and pay 7 TLS
handshakes.

#### What reqwest blocking actually offers (verified, and not what it looks like)

An earlier draft of this plan recommended `.connect_timeout()` + `.read_timeout()`
— *bound stalls, not total duration*. **That fix does not exist here.** Verified
against reqwest 0.12.28:

- **`read_timeout` is not exposed on blocking at all.** It is an async-only
  builder method (`async_impl/client.rs:1453`); nothing in `src/blocking/`
  mentions it.
- **`tcp_user_timeout`** — the one true stall detector available — is
  `#[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]`
  (`blocking/client.rs:728`), so it does not exist on macOS. Not portable.
- **Blocking `timeout` is a deadline, applied *twice*.** Its doc says "connect,
  read and write operations", which reads like a per-operation bound. It is not:
  `execute_request` wraps the connect+headers future in `wait::timeout(f,
  timeout)` (`blocking/client.rs:1438-1456`), then hands the same duration to the
  `Response`, and `blocking::Response::bytes` re-wraps the **whole body read** in
  a *fresh* `wait::timeout(self.inner.bytes(), self.timeout)`
  (`blocking/response.rs:267-268`). So the default is not one 30s budget — it is
  30s for headers and then another 30s for the entire body.

So the bug is real but its mechanism is narrower than "one 30s whole-request
cap": **the body read gets its own fresh 30s**, and a blob that cannot transfer
inside it fails. 64 MiB inside 30s demands ~18 Mbit/s sustained — the common
case, not an edge case.

#### The fix that is actually available

There is **no stall timeout** in blocking reqwest, so the honest choice is between
an unbounded hang and a generous deadline. Take the deadline:

- one shared client with `.connect_timeout(10s)` (reqwest leaves this unset by
  default — a dead host otherwise burns the full request deadline) and
  `.timeout(30s)`, preserving today's control-plane behavior, which is correct for
  small JSON payloads that should fail fast;
- `fetch_blob` overrides **per request** with `RequestBuilder::timeout`
  (`blocking/request.rs:360`), which rides into the `Response` via
  `req.timeout().copied().or(self.timeout.0)` (`blocking/client.rs:1438`) and so
  covers the body read, not just the headers. 10 minutes carries a full 64 MiB
  down to ~0.9 Mbit/s while still bounding a wedged socket.

A true stall timeout needs either the async API or a chunked reader with
per-chunk deadlines. Record that as the known limitation rather than pretending
the deadline is one.

### 4.2 Publish: blobs first, `.mfp` last

In `publish_package_project` (`pkg.rs:121-218`), between the build (`:140`) and
the existing `POST /validate` (`:186`):

1. parse the built `.mfp` (already done at `:153`) and read its section-10 table;
2. for each `vendor` locator: `HEAD /blob/<hash>` → skip if present;
3. otherwise read `<project root>/vendor/<source>` and `PUT /blob/<hash>` with the
   raw bytes and the session token (plan-48-A §4.3);
4. proceed to the existing validate → publish.

The file read here is the same one plan-46-B already hashed at build time; the
hash in section 10 **is** the upload's target key, so no fresh hashing is needed
and none should be added — a second hash computation is a second chance to
disagree.

`HEAD`-then-skip is the whole dedup win: an unchanged library uploads once, ever,
across every future version of every package that vendors it (the blob store is
global and content-addressed — plan-48-A §2.2).

Report progress. A silent multi-minute publish looks like a hang; `plan-36`
established `-q`/`-v` conventions for build output and publish should read
consistently with them.

### 4.3 Install: where the bytes land

Downloaded vendor files go to **`<project>/packages/<name>.vendor/<source>`** — a
sibling of `packages/<name>.mfp`, one directory per package.

Three things force this shape:

- **Not `<project>/vendor/`.** That directory belongs to the *consumer's own*
  `libraries` section (plan-46-A §4.2). Writing an imported binding's blobs into
  it would let the tool silently overwrite a file the user placed by hand.
- **Not `packages/<name>/vendor/`.** `packages/<name>/` is already the source-form
  dependency layout (`pkg.rs:1313-1318`). `<name>.vendor/` sits beside
  `<name>.mfp` without colliding with either.
- **Per-package, not flat.** Two different packages may each vendor a file named
  `libfoo.so` with *different bytes* — plan-46-A's uniqueness rule is per-manifest
  and cannot prevent it (§5). A flat directory would have them overwrite each
  other.

This **supersedes plan-46-C §4.4 for imported packages.** The resolution root
becomes:

| whose locator | vendor file read from |
| --- | --- |
| the project's own `libraries` section | `<project>/vendor/<source>` (unchanged) |
| an imported binding's section 10 | `<project>/packages/<name>.vendor/<source>` |

Both keep plan-46-C's verify against the section-10 hash — only the directory
changes. plan-46-C §4.4 and plan-46-D §4.5 (the copy source) need amending to say
so.

### 4.4 Install: fetch, verify, place

In `pkg add` (`pkg.rs:513-582`) and `pkg install` (`resolve.rs:89-145`), **after**
`install_verified_package` succeeds:

1. read section 10 from the now-verified `.mfp`;
2. for each `vendor` locator, `fetch_blob(repo_url, hash)` — which already
   re-hashes (`client.rs:981`), so a corrupt or substituted blob fails there;
3. a `404` → `PACKAGE_VENDOR_BLOB_MISSING`; a hash mismatch →
   `PACKAGE_VENDOR_BLOB_HASH_MISMATCH`;
4. write to `packages/<name>.vendor/<source>` using the **same stage-verify-rename
   discipline** as `install_verified_package` (`mod.rs:21-49`): an exclusively
   created (`create_new(true)`) `.part` file, verified, then renamed. bug-27 was
   exactly this class of bug; do not open-code a weaker version of it.

Per the settled decision, `pkg add` downloads **every** vendor blob in the table,
not just the host target's, so a later cross-compile and an offline build both
work. Note the cost honestly in progress output — a 6-slot binding is 6 large
downloads.

**Validate `source` before it becomes a path.** It arrives from the `.mfp`, which
is untrusted input. plan-46-B §4.1 already requires the section-10 decoder to
re-check the bare-filename rule (no separators, no `..`, no NUL); this plan
depends on that check and must not assume it. `validate_package_name`
(`src/manifest/package.rs:113-125`) governs `<name>`, not `<source>`, and its
charset is the wrong rule here anyway.

New diagnostics (`src/rules/table.rs`, next free per §2.4):
- `6-605-0010` `PACKAGE_VENDOR_BLOB_MISSING` (Error) — the registry has no blob
  for a hash section 10 names.
- `6-605-0011` `PACKAGE_VENDOR_BLOB_HASH_MISMATCH` (Error) — a downloaded blob's
  sha256 does not match the signed table.

## 5. Finding: vendor filenames can collide across packages

Surfaced while designing §4.3, and **not** a plan-48 bug — a pre-existing gap in
plan-46 that this plan makes easy to hit.

plan-46-A §4.3 requires vendor `source` filenames to be unique **project-wide**,
meaning within one manifest's `libraries` section. Nothing makes them unique
**build-wide**. So:

- package `A` vendors `libfoo.so` (bytes X); package `B` vendors `libfoo.so`
  (bytes Y);
- a consumer imports both; both locators resolve for the target;
- plan-46-D §4.5 copies both into one `<outdir>/<name>/vendor/` → **one
  overwrites the other**;
- plan-46-C emits `dlopen("libfoo.so")` for both logical libraries → whichever
  file survives, one binding loads the wrong library.

This is silent and produces a wrong-library load — the exact failure mode plan-46
exists to eliminate. `<name>.vendor/` per-package directories (§4.3) keep the
*download* side safe, but the collision reappears at plan-46-D's copy step, which
must flatten into one directory for a single RPATH to work.

**Resolved in plan-46 — see plan-46-D §4.5 and plan-46-C §3.1.** The analysis
belongs there, since that is where the copy flattens and the name is emitted; this
section records only that plan-48 is what makes the case easy to hit.

The resolution in brief: vendor files are copied — and their `dlopen` name
emitted — as **`<declaring-unit>-<source>`** (`sqlite3-libfoo.so`), so two
packages shipping a same-named library never collide. Erroring on the collision
was rejected: it would make two unrelated packages permanently un-combinable with
*neither author able to fix it*. A residual `NATIVE_LIBRARY_VENDOR_COLLISION`
(`2-203-0122`) guards the one pathological case the prefix cannot cover (a project
sharing a name with one of its own dependencies) and should never fire.

Nothing is required of **this** plan: §4.3's `packages/<name>.vendor/` directories
are already per-package, so the download side has no collision. The prefix is
applied where the flattening happens — plan-46-D's copy into the single output
`vendor/` — not to the stored file.

## Compatibility / Format Impact

- **Behavior:** `pkg publish` gains upload round trips; `pkg add`/`pkg install`
  gain downloads — both only for packages with section-10 `vendor` locators. A
  package with none behaves exactly as today.
- **On disk:** a new `packages/<name>.vendor/` directory appears only when a
  package vendors something.
- **No format change:** no `.mfp`, no `mfb.lock` (§3.2), no protocol (plan-48-A
  owns that).
- **Transport:** the shared client and timeout change (§4.1) affects *every* repo
  call, not just blobs. That is the point — the 30 s cap is wrong for the `.mfp`
  path too — but it means the artifact/acceptance gates should be watched.

## Phases

### Phase 1 — transport hardening — **DONE**

Independently valuable and a strict prerequisite; fixes a latent bug for every
existing call. Landed ahead of Phases 2-3, which need plan-48-A's endpoints.

- [x] Build one shared `reqwest::blocking::Client` with `connect_timeout(10s)` +
      `timeout(CONTROL_TIMEOUT)` per §4.1, behind a `OnceLock`; replace all three
      `Client::new()` sites (`client.rs:961`, `:1045`, `:1056`).
- [x] Give `fetch_blob` a per-request `BLOB_TIMEOUT` override (§4.1) — **not** a
      `read_timeout`, which blocking reqwest does not expose.
- [x] Tests: `http_client_is_built_once_and_shared` (pointer identity, so
      connection reuse is real); `fetch_blob_survives_a_body_slower_than_the_control_timeout`
      — a loopback server that withholds the body past `CONTROL_TIMEOUT`, which is
      exactly what the old code rejected. Necessarily ~32s: the 30s bound it
      guards *is* the thing under test.

Acceptance: a blob whose body arrives after 30 s downloads successfully, where it
failed before. Verified by the regression test against a real socket.
Commit: 09d6efcd

### Phase 2 — publish uploads vendor blobs

- [x] Add a `put_blob(repo_url, hash, bytes, session_token)` client helper — the
      first `PUT` in the codebase (§2.2) — and a `blob_exists(repo_url, hash)`
      `HEAD` helper.
- [x] Wire §4.2 into `publish_package_project` (`pkg.rs:121-218`) between build
      and `POST /validate`: read section 10, HEAD each, PUT the missing, report
      progress.
- [x] Tests: publishing a binding with vendor locators uploads exactly the blobs
      the registry lacks and **skips** the ones it has (assert the HEAD/PUT call
      pattern against a local `mfb-repo`); a publish whose blob upload fails does
      not publish the `.mfp`; a package with no vendor locators makes **zero**
      extra requests (regression).

Acceptance: an end-to-end `pkg publish` of a vendoring binding against a local
`mfb-repo` lands every blob and the `.mfp`, and re-publishing a new version with
an unchanged library uploads **no** blob bytes.
Commit: 0cb8186a

### Phase 3 — install downloads and verifies

- [x] Implement §4.4 in `pkg add` (`pkg.rs:513-582`) and `pkg install`
      (`resolve.rs:89-145`), after `.mfp` verification, using stage-verify-rename
      (`mod.rs:21-49`).
- [x] Add `6-605-0010`/`0011` to `src/rules/table.rs`.
- [x] Amend plan-46-C §4.4 and plan-46-D §4.5 for the imported-package resolution
      root (§4.3) — the prefix itself is already specified in plan-46-D §4.5 and
      needs nothing from this plan (§5).
- [x] Tests: `pkg add` of a vendoring binding leaves verified files in
      `packages/<name>.vendor/`; a registry serving a **tampered** blob fails with
      `PACKAGE_VENDOR_BLOB_HASH_MISMATCH` and leaves **no** file (not even a
      `.part`); a missing blob fails with `PACKAGE_VENDOR_BLOB_MISSING`; an
      unsigned package's blobs are fetched but not reported as verified (§3.3); a
      `source` carrying a path separator or `..` is refused before any file
      operation.
- [x] Doc: update `src/docs/spec/tooling/` (the `packages/` layout gains
      `<name>.vendor/`), `src/docs/spec/package-manager/01_repository-protocol.md`
      (client flow), and `src/docs/spec/language/17_native-libraries.md` (vendor
      libraries now arrive with the package — remove plan-46-C's "distribution is
      deferred" note).

Acceptance: on a machine that has never seen the library, `mfb pkg add` followed
by `mfb build` produces a running executable that `dlopen`s the vendored library
— **with no file placed by hand.** That is the acceptance for the whole plan-46 +
plan-48 arc, and it needs a real end-to-end run, not a unit test.
Commit: 0cb8186a

## Validation Plan

- Tests: per phase; the throttled-download regression (Phase 1) is the one most
  likely to be skipped and the one that matters most.
- Runtime proof: **required** — a real `mfb-repo` (both local-datapath and MinIO
  S3 modes), a real vendoring binding, publish from one directory, `pkg add` into
  a clean project elsewhere, build, and run. Confirm the tampered-blob case fails
  closed by corrupting the stored blob and re-adding.
- Doc sync: the three spec files above; `.ai/specifications.md` obligation.
- Acceptance: `scripts/test-accept.sh` green; `tests/repo_acceptance.rs` covers
  the CLI-facing registry surface and should gain the vendor cases.

## Open Decisions

- **A shared blob cache.** Every `fetch_blob` is a fresh download; nothing is
  cached across projects (§2.3). Ten projects importing the same binding download
  the same 40 MB library ten times. `~/.mfb/<sha256(repo_url)>/` is the obvious
  home (repo-scoped, private perms, already exists) and the blobs are
  content-addressed and immutable, so a cache is nearly free correctness-wise.
  Recommend: **out of scope here, filed immediately after.** It changes no
  protocol and is a pure win, but it is a distinct concern from getting the bytes
  to move at all.
- Whether to parallelize the blob downloads. They are serial today
  (`resolve.rs:122`), and a 6-slot binding is 6 large sequential fetches. The
  shared client (§4.1) makes concurrency cheap to add later. Recommend: serial
  first, measure, then decide — correctness before throughput.
- Whether `pkg add file://` (`pkg.rs:458-505`) should carry vendor blobs at all.
  It is a local copy with no registry, and it already skips
  `install_verified_package` entirely, pinning trust-on-first-use. Recommend: no
  blob fetching (there is no registry to fetch from) and an explicit error if the
  `.mfp` names vendor locators, rather than a silently unusable install.

## Summary

The download half is nearly free: `fetch_blob` is already hash-addressed,
redirect-following, and self-verifying, and that hash check is the entire security
argument — it holds for any content. The upload half is genuinely new (the client
has never issued a `PUT`), but small.

The real prerequisite was transport, and it is **already done** (§4.1): reqwest
blocking defaults to a 30s deadline that — contrary to both its own doc and this
plan's first draft — is applied *twice*, giving the body read its own fresh 30s.
Any blob too slow for that failed outright, a latent bug for large `.mfp`s today.
Blocking reqwest exposes no stall timeout (`read_timeout` is async-only,
`tcp_user_timeout` is Linux-only), so the fix is a shared client with a bounded
connect phase plus a generous per-request deadline for `/blob` — a known
limitation, recorded rather than papered over.

Two things this plan surfaced rather than solved, both now resolved elsewhere:
vendor filenames can **collide across packages** (§5 — fixed in plan-46-D §4.5 by
prefixing the copied file and its `dlopen` name, rather than erroring), and blob
storage is **permanent with no GC** while this plan makes it large and
orphan-able (→ plan-49). The remaining gap is the absence of a **blob cache**, so
N projects re-download the same library N times — an Open Decision here, and a
pure win whenever someone takes it.
