# bug-908: FUNCTION_TABLE parameter whose `paramFlags` and `defaultConst` disagree is accepted — ABI hash says "has default", importer says "no default"

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Coordinator note: the consumer build was re-run by the coordinator; the arity/ABI-hash half was measured by the review agent only.
Regression Test: src/binary_repr/reader.rs unit test (proposed: reject `flags & HAS_DEFAULT == 0 && defaultConst != 0xFFFFFFFF`)

The spec defines `defaultConst` as "a CONST_POOL index for a literal default, the
FUNCTION_TABLE index of the parameter's hidden default function when bit 3 is set, or
`0xFFFFFFFF` when the parameter has no default", with bit 0 = "has default". The reader
validates bit 3 strictly (`validate_default_functions`) but never checks that bit 0 and
`defaultConst` agree. Two consumers then read the record differently:

- `src/binary_repr/sections.rs:function_sig_hash` decides "has a literal default" from
  `param.default_const != u32::MAX` (ignoring bit 0);
- `src/binary_repr/builder.rs:export_default` decides it from `param.flags & PARAM_FLAG_DEFAULT`.

A record with `paramFlags = 0, defaultConst = 0` therefore keeps the original ABI hash
(the one computed for `b AS Integer = 10`) and passes `validate_abi_index`, while every
importer sees `b` as required. The ABI hash — the value `mfb repo check-abi` and the
resolver use to prove two versions expose the same signature — no longer identifies the
importer-visible signature: removing a default (an ABI break) is invisible to it.

Correct behavior: the reader rejects a parameter record whose bit 0 is clear but whose
`defaultConst` is not `0xFFFFFFFF` (and vice versa), or both consumers derive "has
default" from the same field.

References:

- `src/docs/spec/package/07_functions.md` — "Parameters": `defaultConst ... or 0xFFFFFFFF when the parameter has no default`; bit 0 = has default.
- `src/docs/spec/package/03_metadata-encoding.md` — function hash: "a default byte: `0` for no default; `1` followed by the serialized default constant ...".
- `src/docs/spec/package/12_verifier-rules.md` — "`validate_abi_index` ... recomputes each callable hash from its function metadata".

## Failing Reproduction

`patch_mfp.py` is in `/tmp/hardening/B10/repro/` (replaces bytes in one section, fixes the
section table and payload hash).

```
cd /tmp/hardening/B10/repro
mfb init-pkg dpkg
printf 'EXPORT FUNC add(a AS Integer, b AS Integer = 10) AS Integer\n  RETURN a + b\nEND FUNC\n' > dpkg/src/lib.mfb
(cd dpkg && mfb build)
mfb init dcons && cd dcons && mfb pkg add file:///tmp/hardening/B10/repro/dpkg/dpkg.mfp
printf 'IMPORT io\nIMPORT dpkg\nFUNC main() AS Integer\n  io::print(toString(dpkg::add(1)) & " " & toString(dpkg::add(1, 2)))\n  RETURN 0\nEND FUNC\n' > src/main.mfb
mfb build && ./build/dcons.out                 # 11 3
# param b record (name=4, type=3, flags=1, defaultConst=0) -> flags=0, defaultConst still 0
python3 ../patch_mfp.py ../dpkg/dpkg.mfp packages/dpkg.mfp 8 04000000030000000100000000000000 04000000030000000000000000000000
mfb build
python3 /tmp/hardening/B10/mfp.py packages/dpkg.mfp | grep '^ABI'
```

- Observed: the package is accepted (`uses dpkg - [Unsigned]`, no IMPORT_PACKAGE_INVALID);
  the call fails as `error[2-203-0022 TYPE_CALL_ARITY_MISMATCH] Call to dpkg.add has 1 argument(s), expected 2 to 2.`;
  the stored and re-validated ABI hash is unchanged: `ABI 1 0 [('add', 1, '68c06809a5894c8c')]`
  (identical to the unpatched package with the default). `dpkg::add(1, 2)` alone builds and prints `3`.
- Expected: `failed to read ... parameter b ... defaultConst set without the default flag` (IMPORT_PACKAGE_INVALID).

Contrast: `paramFlags = 1, defaultConst = 0xFFFFFFFF` is rejected
(`PACKAGE_INVALID ... unreadable or invalid function metadata`); bit 3 without bit 0 is
rejected by `validate_default_functions`.

## Root Cause

`src/binary_repr/reader.rs:validate_default_functions` only inspects records with
`PARAM_FLAG_DEFAULT_FUNCTION`; no check ties `PARAM_FLAG_DEFAULT` to `default_const`.
`function_sig_hash` and `export_default` use different fields as the "has default" test.

## Goal

- Reader rejects any parameter record where `(flags & PARAM_FLAG_DEFAULT != 0) != (default_const != u32::MAX)`.

### Non-goals (must NOT change)

- The ABI hash input for well-formed packages (no rehash of existing packages).

## Blast Radius

- `function_sig_hash` vs `export_default` — fixed by this bug
- reserved paramFlags bits 1–2 are also accepted silently (patched flags=6 builds) — same check site
