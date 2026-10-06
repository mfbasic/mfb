# bug-948: owner names of 241–255 bytes pass validation but cannot register (246+) or ever rotate their ident (241–245)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: (proposed) repository/src/client.rs `max_length_owner_can_register_and_rotate`

`OWNER_LIMIT` is 255 bytes, but the key store names files `<owner>.<role>.<ext>`
inside `keys/`, and a path component is capped at 255 bytes (NAME_MAX on macOS and Linux).
The longest suffix, `.ident.next.prv`/`.ident.next.pub`, adds 15 bytes and `.ident.prv`
adds 10. So:

- 246–255 bytes: `mfb repo register` writes `<owner>.auth.*` (length+9 fits), then fails writing
  `<owner>.ident.pub` with `File name too long`. The auth files are left behind, and every retry
  is then refused with "keys already exist locally".
- 241–245 bytes: registration succeeds (server and client), but `mfb key rotate` always fails
  writing `<owner>.ident.next.pub`. Ident rotation is the documented remedy for a stolen
  machine ("revoke the machine's auth key **and** rotate the ident"), so these accounts can
  never recover except through the operator re-anchor ceremony.

The correct behavior: every owner name the grammar accepts can register, auth, link and rotate. One fix is
to lower `OWNER_LIMIT` to 240 (255 − 15). Another is to change the key-file naming (for
example, hash long owners).

References:

- src/docs/spec/package-manager/04_owner-names.md "Grammar": "`OWNER_LIMIT` is **255**", "max length | `OWNER_LIMIT` = 255 bytes".
- 02_key-store.md "Layout": `keys/<owner>.ident.next.pub` / `.prv`; "All key file names are `format!("{owner}.{role}.ext")`".
- 01_repository-protocol.md "Ident Rotation".

## Failing Reproduction

Local registry, MFB_HOME under /tmp (macOS APFS; Linux ext4 has the same 255-byte NAME_MAX):

```
mfb repo register "L$(python3 -c "print('a'*254)")"   # 255 bytes
mfb repo register "L$(python3 -c "print('a'*245)")"   # 246 bytes
O="L$(python3 -c "print('a'*244)")"                   # 245 bytes
mfb repo register "$O"; mfb repo auth "$O"; mfb key rotate "$O"
```

- Observed: 255 and 246 → `error: failed to write '…/keys/Laaa…aaa.ident.pub': File name too long`
  (the 246 case leaves `<owner>.auth.pub`/`.auth.prv` behind). 245 → `Registered owner …` and
  `Authenticated owner …`, then `mfb key rotate` → `error: failed to write
  '…/keys/Laaa…aaa.ident.next.pub': File name too long`.
- Expected: all three register, and the 245-byte owner rotates.

Contrast: a 240-byte owner (`.ident.next.prv` = 255 bytes) works.

## Root Cause

`wire/src/validation.rs:OWNER_LIMIT` (255) does not account for the
`repository/src/local.rs` path scheme (`auth_public_key_path`, `ident_private_key_path`,
`pending_ident_*` paths), which appends up to 15 bytes to the owner in one path component.

## Goal

- Either `OWNER_LIMIT ≤ 240` (spec updated) or file names that stay ≤ 255 bytes for any valid owner.

### Non-goals (must NOT change)

- Existing registered owners ≤ 240 bytes and their on-disk paths.

## Blast Radius

- `local.rs` key paths (auth/ident/next) and `session/<owner>.ses` (+4, fits) — the 15-byte suffix is the binding one.
- Server side has no filename dependency; the registry accepts 255-byte owners.
