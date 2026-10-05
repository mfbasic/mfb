# bug-949: an empty `MFB_HOME` (or empty `HOME`) puts the private key store in the current directory

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Footgun

Status: Open
Regression Test: (proposed) src/cli/mod.rs `local_paths_for_repo_rejects_empty_mfb_home`

`local_paths_for_repo` (and `LocalPaths::from_env`) take `MFB_HOME` whenever it is
**set**, including when it is set to the empty string, and use it verbatim. The store
root then becomes the relative path `<repo-hash>/`, resolved against the current working
directory. With `HOME=""` (and no `MFB_HOME`) the root becomes the relative `.mfb/<repo-hash>/`.
`mfb repo register` then writes `keys/<owner>.auth.prv` and `<owner>.ident.prv` (the
account identity) into whatever directory the user is in, typically a project checkout.
From there they are one `git add .` away from being published. Empty variables are common
(`MFB_HOME= mfb …`, CI templates that export unset values as `""`).

The correct behavior: an empty `MFB_HOME` is treated as unset (fall back to `HOME`), and an
empty `HOME`/`USERPROFILE` is treated as not set (`HOME is not set`). Equivalently, the
store base must be an absolute path.

References:

- src/docs/spec/package-manager/02_key-store.md "Root Directory": `MFB_HOME` "taken verbatim as the store base"; `HOME` "store base is `$HOME/.mfb`"; "none set — hard error: `HOME is not set`". "The store is private to the invoking user."

## Failing Reproduction

```
mkdir /tmp/x && cd /tmp/x
env -i PATH=/usr/bin:/bin MFB_HOME= HOME=/tmp/somehome MFB_REPO_URL=http://127.0.0.1:47716 \
    /tmp/hardening/mfb-head repo register hank
find . -type f
```

- Observed:
  ```
  Registered owner hank with auth fingerprint 776dbe6e…
  ./<repo-hash>/server.pub
  ./<repo-hash>/keys/hank.auth.prv
  ./<repo-hash>/keys/hank.ident.prv
  ./<repo-hash>/keys/hank.ident.pub
  ./<repo-hash>/keys/hank.auth.pub
  ```
  With `env -i … HOME= …` (MFB_HOME unset), `mfb repo auth x` creates `./.mfb/<repo-hash>/server.pub` in the cwd.
- Expected: keys written under `/tmp/somehome/.mfb/<repo-hash>/` (empty MFB_HOME ignored), or a hard
  error; with `HOME=` → `error: HOME is not set`.

Contrast: `env -i PATH=… MFB_REPO_URL=… mfb repo auth alice` (both unset) → `error: HOME is not set`.

## Root Cause

`src/cli/mod.rs:local_paths_for_repo` and `repository/src/local.rs:LocalPaths::from_env` use
`if let Ok(home) = std::env::var("MFB_HOME")` and `std::env::var("HOME")`. Neither checks for
an empty value or a relative path.

## Goal

- Empty `MFB_HOME` → fallback. Empty `HOME`/`USERPROFILE` → "HOME is not set". Spec row updated to say so.

### Non-goals (must NOT change)

- A non-empty `MFB_HOME` keeps being used verbatim (not joined with `.mfb`).

## Blast Radius

- `src/cli/mod.rs:local_paths_for_repo` (all CLI paths) — fixed by this bug.
- `repository/src/local.rs:LocalPaths::from_env` — same pattern; fix together.
