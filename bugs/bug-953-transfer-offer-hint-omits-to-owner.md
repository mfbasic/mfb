# bug-953: `mfb repo transfer` tells the recipient to run a `transfer-accept` command that always fails

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other

Status: Open
Coordinator note: measured by the review agent against a local `mfb-repo` on 127.0.0.1 only; the coordinator did not re-run it.
Regression Test: (proposed) src/cli/pkg.rs test on the offer message

After a successful offer, `mfb repo transfer <ident> <to-owner>` prints
"they must run `mfb repo transfer-accept <ident>` to accept". `transfer-accept` requires
`<owner>#<package>@<to-owner>` and rejects the printed form. The correct message names
`mfb repo transfer-accept <ident>@<to-owner>`.

References:

- src/docs/spec/tooling/07_cli-reference.md: `mfb repo transfer-accept <owner>#<package>@<to-owner>`.
- `mfb repo --help`: `repo transfer-accept <owner>#<pkg>@<to-owner>`.

## Failing Reproduction

```
mfb repo transfer 'Alice#toolbox' carl
mfb repo transfer-accept 'Alice#toolbox'      # exactly as printed
```

- Observed:
  ```
  Offered Alice#toolbox to carl; they must run `mfb repo transfer-accept Alice#toolbox` to accept.
  error: use <owner>#<package>@<to-owner> to name the accepting account
  ```
  (exit 1)
- Expected: the hint reads `mfb repo transfer-accept Alice#toolbox@carl`, which works
  (`Accepted transfer of ALICE#toolbox to carl.` for the analogous call).

## Root Cause

`src/cli/pkg.rs:transfer_offer` formats `response.ident` without `@{response.to_owner}`.

## Goal

- The printed command is runnable verbatim.

## Blast Radius

- `src/cli/pkg.rs:transfer_offer` only.
