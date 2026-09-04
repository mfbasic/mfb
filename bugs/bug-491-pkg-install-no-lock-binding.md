# bug-491: `mfb pkg install` never binds the fetched blob to the lockfile's ident/version

Last updated: 2026-09-03
Effort: small (<1h)
Severity: MEDIUM (constrained)
Class: security (supply-chain integrity)

Status: Open (found in audit-3, Surface 9 SUP-04; `planning/completed/audit-3-supply-chain.md`)

Regression Test: none yet — add one asserting install rejects a blob whose header ident/version/name differs from the lock.

## Summary

`mfb pkg install` fetches a package blob by content hash and runs the full
plan-23 §3.5 signature chain on it, but §3.5 only proves the artifact is
*self-consistent and signed by the pinned owner* — nothing compares the
installed header to the lock's `ident` / `selected` / `name`. The sibling
`mfb pkg add` path *does* perform that comparison (`src/cli/pkg.rs:1226-1233`,
`header.ident != full_ident`), and the one place a mismatch could otherwise
surface is dead as a gate. Combined with the unsigned `/index` version list
(SUP-01 / bug-189), a registry that maps `version → hash` freely can serve any of
the owner's artifacts for a requested version and have it accepted at install.

## Mechanism

```rust
// src/cli/resolve.rs:415
let blob = client::fetch_blob(&repo_url, &package.hash)?;
super::install_verified_package(&packages_dir, &package.name, &blob,
                                Some(&package.ident_key))
```

`classify_installed_package` (`src/cli/build/packages.rs:155-243`) then checks
identKey == pin, attestation, proof, signature, payload hash — all of which
`verify_attestation` (`repository/src/package.rs:278-306`) binds to
`package.ident`/`package.version` read out of *the same file*. Nothing compares
the installed header to the lock's `ident`/`selected`.

The would-be gate returns its error into a discarded `Result`:

```rust
// src/manifest/package.rs:366
if dependency.pin && header.version != dependency.version {
    return Err(format!("package `{}` is pinned to version {} ...", ...));
}
```

```
$ grep -rn 'installed_package_files' src/
# three callers, all: `let Ok(packages) = installed_package_files(..) else { return ... };`
#   src/manifest/package.rs:409, :471, :558
```

so the pinned-version error never reaches the build; a floating dependency gets
no version comparison at build at all.

## Constraint (why MEDIUM, not HIGH)

To be *silently* accepted, the substituted artifact must be signed by the same
owner's pinned key **and** carry the same internal package `name` — the resolver
loads `packages/<declared name>.mfp` (`src/resolver/packages.rs:89`) and prefixes
symbols by the artifact's own name, so a `name` mismatch usually degrades to a
link/type failure rather than a clean substitution. It remains the designed
enforcement point, and the sibling `pkg add` performs exactly the check that is
missing here.

## Reproduction

Not demonstrated: requires a registry able to sign a name binding. The missing
comparison is a direct read of `install` vs `add_package_from_registry`.

## Best fix

In `install` (`src/cli/resolve.rs:415`), parse the fetched blob with
`mfb_repository::package::parse_mfp_package` before staging and refuse unless
`header.ident == package.ident && header.version == package.selected &&
header.name == package.name` — the same three-way comparison
`package_dependency_status` already implements for `mfb pkg verify`
(`src/cli/pkg.rs:2167`). Separately, either route
`installed_package_files`' pinned-version error to the build (a call from
`verify_and_report_packages`) or delete the check so it stops reading as an
enforced gate.

## Non-goals

- Compare against the lock's `selected`, not the manifest's `version` (which is
  an ABI floor for a `pin: false` dependency).
- Do not break `file://` / source-directory dependencies, which have no lock
  entry.

## Prior art

None for the install-side binding (searched ident-mismatch / substitut /
lockfile / selected). audit-2 SUP-04 documented install as "not blind" — still
true; this narrows *which* property install actually proves. Related: SUP-01 /
bug-189 (unsigned `/index`), which supplies the `version → hash` freedom this
depends on.
