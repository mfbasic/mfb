# bug-794: vendor installation skips required filenames when locators share a hash

Last updated: 2026-10-02
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness
Status: Open
Regression Test: proposed `tests/cli/cli_repo_install.rs:spec_probe_identical_vendor_blobs_keep_all_filenames`

Every vendor locator's source filename must exist after a successful package
installation, even when distinct filenames contain identical bytes. Downloads
may be deduplicated by hash; file placement may not. Currently `pkg add` reports
success but writes only the first filename for a repeated hash. A later build
or cross-build that needs another locator cannot find its library.

Found during the package-manager spec review. The existing GC acceptance test
explicitly gives every vendor file different bytes to avoid this behavior; that
fixture remains unchanged. The new probe copies its package setup into a new
independent test and uses identical bytes to exercise the missing case.

## Failing Reproduction

From the repository root:

```sh
git worktree add --detach /tmp/mfb-vendor-repro HEAD
```

Run this Python to append a new test to the disposable worktree. No existing test
is changed, weakened, or re-baselined:

```python
from pathlib import Path
p=Path('/tmp/mfb-vendor-repro/tests/cli/cli_repo_install.rs')
s=p.read_text()
a=s.index('#[test]\nfn repo_gc_reclaims_an_orphaned_blob_and_leaves_live_packages_installable()')
b=s.index('    // --- the orphan:',a)
new=s[a:b].replace('repo_gc_reclaims_an_orphaned_blob_and_leaves_live_packages_installable','spec_probe_identical_vendor_blobs_keep_all_filenames')
new=new.replace('|source: &str| format!("gc acceptance vendored library: {source}").into_bytes()', '|_source: &str| b"identical vendor bytes".to_vec()')
new+='''    let app_dir=work.path().join("consumer");
    assert!(run_mfb_plain(&["init",app_dir.to_str().unwrap()]).status.success());
    let add=Command::new(mfb_exe()).args(["pkg","add","alice#gcvendor"])
        .current_dir(&app_dir).env("MFB_REPO_URL",&repo.url)
        .env("MFB_HOME",home.path().join(".mfb")).output().unwrap();
    assert!(add.status.success(),"{}",String::from_utf8_lossy(&add.stderr));
    for source in ["libgc.dylib","libgc.dll","libgc-x86_64-glibc.so"] {
        assert!(app_dir.join("packages/gcvendor.vendor").join(source).is_file(),
            "successful install omitted vendor source {source}");
    }
}
'''
p.write_text(s+'\n'+new)

```

```sh
CARGO_TARGET_DIR=/tmp/mfb-vendor-repro-target cargo test \
  --manifest-path /tmp/mfb-vendor-repro/Cargo.toml \
  --test cli_repo_install spec_probe_identical_vendor -- --nocapture
```

Observed against HEAD `b285741f3` on macOS in `/tmp/mfb-package-spec-head`:

```text
successful install omitted vendor source libgc.dll
test spec_probe_identical_vendor_blobs_keep_all_filenames ... FAILED
```

The measured run used `CARGO_TARGET_DIR=/tmp/mfb-package-spec-target` and
`MFB_TEST_EXE=/Users/justinzaun/Development/mfb/target/debug/mfb`; production source
in that binary matches HEAD (only embedded documentation differs). The local
registry runs the HEAD implementation. No external registry is contacted.
Expected: all named files exist and contain the hash-verified bytes. Other
platforms have not been measured.

## Root Cause

`src/cli/pkg.rs:install_vendor_blobs` inserts each hash into `seen` and executes
`continue` for duplicates before validating or placing the later source path.
Deduplication therefore suppresses both downloading and writing. A content hash
identifies bytes, not the set of required filenames.

The package's signed native-library table retains the distinct source names;
`binary_repr::read_package_native_libraries` returns those locators unchanged.
The installer silently drops obligations from that table.

## Goal

A successful install materializes every required vendor source filename with its
verified bytes, while fetching each distinct hash at most once per installation.

### Non-goals

Do not require publishers to alter identical library bytes, remove valid
locators, change signed artifacts, or weaken hash verification. Do not weaken the
existing GC acceptance test or replace its distinct-byte coverage.

## Blast Radius

- `src/cli/pkg.rs:install_vendor_blobs` — fix; shared by registry add and locked
  install (`src/cli/resolve.rs:install`).
- `src/cli/pkg.rs:upload_vendor_blobs` — unaffected: server blobs are addressed by
  hash, so one upload for identical bytes is correct.
- `repository/src/server.rs:validate_package_request` — unaffected: existence
  probes may deduplicate hashes; diagnostics still enumerate locators.
- `src/cli/mod.rs:install_vendor_file` — preserve its safe staging/rename and
  filename checks when placing each destination.
- Native-library lookup at build time — consumer of every declared source name;
  include a cross-target lookup regression, not just directory counts.

## Fix Design

Separate hash-based fetch caching from destination placement. Each distinct
source destination must be validated and installed even on a cache hit. Detect
conflicting mappings where a source filename is assigned different hashes;
never silently overwrite one signed requirement with another. Retain safe
staging/rename and bounded memory use for large library blobs.

## Phases

### Phase 1 — failing tests and audit

- [ ] Add independent same-hash/different-source coverage for add and install.
- [ ] Audit destination collisions, same-source duplicates, and cross-target
      lookup expectations.

Acceptance: new tests fail for omitted filenames; existing fixtures unchanged.
Commit: —

### Phase 2 — implementation

- [ ] Separate download deduplication from file placement.
- [ ] Update the package-manager spec to describe the corrected guarantee.

Acceptance: every filename is placed, each hash fetched at most once, and
hash/filename security checks still hold.
Commit: —

### Phase 3 — full validation

- [ ] Run the full repository and root suites, including registry install,
      publish, and GC integration tests.
- [ ] Re-run the reproduction and cross-target native-library lookup cases.

Acceptance: the install guarantee holds end to end without weakened tests.
Commit: —
