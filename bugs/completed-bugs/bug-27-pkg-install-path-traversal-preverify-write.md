# bug-27: Package install writes an untrusted `name`/`hash`-derived path with `fs::write` BEFORE verification → path traversal + symlink-follow arbitrary write

Last updated: 2026-07-08
Effort: medium (1h–2h)

The package-install paths take an **untrusted** string from a downloaded/locked
package and use it as a filesystem path component, then `fs::write` the fetched
blob there **before** any signature/attestation verification. `fs::write` follows
symlinks, and the untrusted string is never checked for `/`, `\`, or `..`, so a
crafted package can (a) escape `packages/` (path traversal) and (b) overwrite the
target of a pre-planted symlink — with attacker-controlled bytes, pre-verification.

Three sites share the pattern:

1. **`mfb pkg add <owner>#<package>` — HIGH.** `add_package_from_registry`
   (`src/cli/pkg.rs:544-549`) computes
   `destination = packages_dir.join(format!("{}.mfp", header.name))` and
   `fs::write(&destination, &blob)`. `header.name` comes from
   `parse_mfp_package(&blob)` on the registry-served blob; `read_mfp_string`
   (`src/manifest/package.rs:202-215`) validates only UTF-8 + length + non-empty —
   **never** rejects path separators or `..`. The package `ident` IS checked
   against `owner#package` (`pkg.rs:537`), but `name` is free-form and unchecked.
   Verification (`classify_installed_package`) runs only at `pkg.rs:554-556`, after
   the write. `add_package_from_file` (`pkg.rs:462,480-481`) has the same
   untrusted-`name`→join→copy pattern from a local `.mfp`.
2. **`mfb pkg install` (locked) — MEDIUM.** `install` (`src/cli/resolve.rs:122-130`)
   does `packages_dir.join(format!("{}.mfp", package.name))` + `fs::write` before
   `classify_installed_package` (`:127-128`). `package.name` is read verbatim from
   `mfb.lock` (`read_lock`, `resolve.rs:~619`) — a file an attacker who ships a
   repo controls.
3. **Resolver blob staging — MEDIUM.** `load_import_edges`
   (`src/cli/resolve.rs:415-419`) stages to
   `std::env::temp_dir().join(format!("mfb-resolve-{hash}.mfp"))` + `fs::write`.
   The name is fully predictable (`hash` is the public content hash) in a shared
   `/tmp`; an attacker pre-plants that path as a symlink to a victim file and
   `fs::write` clobbers it (symlink/TOCTOU). A non-hex `hash` from the registry
   index also escapes `temp_dir`.

The single correct behavior a fix produces: no untrusted string is used as a path
component without charset validation, and no attacker-controlled bytes are written
to a computed path before verification — writes go to an `O_EXCL`/`create_new`
temp file inside the destination dir, are verified, then atomically renamed.

Severity HIGH (site 1: pre-verification arbitrary `.mfp` write outside `packages/`
via `mfb pkg add`, author-controlled on any registry); sites 2–3 MEDIUM (require a
hostile `mfb.lock` or a shared host).

References:

- `src/cli/pkg.rs:544-549` (registry add: unvalidated `header.name` → `join` →
  `fs::write` pre-verify), `:462,480-481` (local-file add, same pattern), `:537`
  (ident IS checked — name is not).
- `src/manifest/package.rs:202-215` (`read_mfp_string` — no path-char rejection).
- `src/cli/resolve.rs:122-130` (locked install pre-verify write), `:415-419`
  (predictable `/tmp` staging + symlink follow + non-hex-hash traversal),
  `:~619` (`read_lock` reads `name` verbatim).
- Contrast: `make_temp_output_dir` (`src/cli/build.rs:674-681`) adds `nanos`
  entropy — the resolver staging name has none.
- Related: audit-1 PKG-01/02 (import trust boundary), REPO-06/07 (untrusted
  owner/hash interpolation). Distinct: this is the *client-side install* write.
- Found during goal-01 review of `src/cli/**`.

## Failing Reproduction

Site 1 (`mfb pkg add`): publish/serve a `.mfp` whose header `name` =
`../../../../home/victim/.config/autostart/x`; victim runs
`mfb pkg add owner#package`:

- Observed: `packages_dir.join("../../…x.mfp")` resolves outside `packages/`, and
  `fs::write` writes the attacker blob there before verification. If that path is a
  symlink, its target is clobbered. On verify failure the escaped path is removed —
  but the write already happened.
- Expected: the crafted `name` is rejected immediately (before any `join`/write),
  and the install refuses.

Contrast: `header.ident` traversal is impossible (checked at `pkg.rs:537`); only
`name` is unchecked. `install`'s repo-fingerprint / pinned-key / project-hash gates
(`resolve.rs:96-117`) are enforced — the pre-verify write is the sole unguarded
step.

## Root Cause

`read_mfp_string` and `read_lock` do not constrain `name` to a safe charset, and
all three install sites `fs::write` (symlink-following) to a name/hash-derived path
before verifying the package. The write precedes the trust check.

## Goal

- No untrusted `name`/`hash` reaches a path component without strict validation
  (`[A-Za-z0-9._-]+`, no separators, no `..`), and no attacker bytes are written to
  a final/computed path before verification (temp `create_new` → verify → atomic
  rename).

### Non-goals (must NOT change)

- The verification chain itself (plan-23 §3.5) — it is correct; only its ordering
  relative to the write and the missing name validation are the bug.
- Legitimate package names.

## Blast Radius

- `pkg.rs:544-549` (registry add) — HIGH, fixed here.
- `pkg.rs:462,480-481` (local-file add) — same pattern, fixed here.
- `resolve.rs:122-130` (locked install) — fixed here.
- `resolve.rs:415-419` (resolver staging) — fixed here (use `O_EXCL` temp + hex
  validation, or decode from the in-memory blob without staging).
- `read_mfp_string` (`manifest/package.rs`) and `read_lock` (`resolve.rs`) — add
  name charset validation at the source so every consumer benefits.

## Fix Design

1. Validate `name` (and any hash used in a path) to a strict charset with no
   separators/`..` immediately after parse (`read_mfp_string` for `.mfp` name,
   `read_lock` for lock name), rejecting on violation.
2. At every install site, write the blob to a `create_new` temp file **inside** the
   destination directory (never a symlink-followable final path), run
   `classify_installed_package` on it, and only on `Verified` atomically rename to
   `packages_dir.join("<name>.mfp")`.
3. For the resolver staging, prefer decoding `read_package_info` from the in-memory
   blob (no disk staging); if a temp file is required, use
   `tempfile::NamedTempFile::new_in` (O_EXCL, unpredictable) and validate `hash` is
   pure hex.

## Phases

### Phase 1 — failing test + audit

- [ ] Add tests: a `.mfp`/lock with `name = "../evil"` is rejected before any
      write; a symlink pre-planted at the destination is not followed. Confirm they
      fail today (write occurs / symlink clobbered).
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Name/hash charset validation at the sources; temp-create_new→verify→rename at
      all four write sites; hex-validate the resolver hash or skip staging.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh`; end-to-end `mfb pkg add`/`install`/`update` against
      a local registry with well-formed and crafted-name packages.

## Validation Plan

- Regression test(s): traversal-name rejection + symlink-non-follow tests at each
  site.
- Runtime proof: `mfb pkg add` of a crafted-`name` package writes nothing outside
  `packages/` and refuses.
- Full suite: `scripts/test-accept.sh` + package-install integration.

## Summary

The engineering risk is ordering (verify before any final-path write) and getting
the name/hash validation strict without breaking legitimate names; the fix is a
charset guard at the parse sources plus a temp→verify→rename discipline at the four
install/staging sites.
