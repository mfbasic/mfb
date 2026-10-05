# bug-848: a `PUBLIC` field of an `EXPORT TYPE` is readable, constructible and `WITH`-updatable by importers

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/syntax/types/<new>-public-field-hidden-from-importer-invalid (proposed)

A package that declares `EXPORT TYPE Secret` with a field marked `PUBLIC` intends that
field to be package-internal: the spec says a `PUBLIC` field "can be used only by files
in the declaring package" and records with such fields are "opaque across that
boundary". Today an importing project can read the field with `.`, construct the record
positionally (setting the hidden field), and update it with `WITH`. Builds succeed with no
diagnostic, and the program reads and writes the package's hidden state. It happens with
both a `local://` source dependency and an installed `packages/<name>.mfp`.
It is silent: encapsulation a package author relied on is simply not enforced.
**Correct behavior:** each of the three accesses from outside the declaring package
fails with `TYPE_MEMBER_NOT_VISIBLE`, the same code a `PRIVATE` field already produces
for read and construct.

Class (brief taxonomy): CODE BUG. The spec is consistent across two topics.

References:

- src/docs/spec/language/13_modules-and-packages.md: "`PUBLIC` (default) — visible to all files in the same package, hidden from importers." and "constructors for records with hidden fields are callable only from scopes that can see every required field."
- src/docs/spec/language/04_types.md: "A `PUBLIC` field can be used only by files in the declaring package. ... Outside code that cannot see a field also cannot set it in a constructor, read it with `.`, or update it with `WITH`"
- Found during hardening batch B03 (topic language/13 modules-and-packages).

## Failing Reproduction

```
mfb init-pkg shapes ; mfb init app2
# shapes/src/lib.mfb
EXPORT TYPE Secret
  visible AS Integer
  PUBLIC hidden AS Integer
END TYPE
EXPORT FUNC makeSecret() AS Secret
  RETURN Secret[1, 2]
END FUNC

# app2/project.json  "packages": [{"name":"shapes","version":"=0.1.0",
#   "source":"local:///tmp/hardening/B03/modules/shapes","direct":true,"requiredBy":[]}]
# app2/src/main.mfb
IMPORT io
IMPORT shapes
FUNC main() AS Nothing
  LET sec = shapes::makeSecret()
  io::print(toString(sec.hidden))            ' read
  LET forged = shapes::Secret[1, 99]         ' construct, setting hidden
  LET t = WITH sec { hidden := 77 }          ' update
  io::print(toString(forged.hidden))
  io::print(toString(t.hidden))
END FUNC

cd app2 && /tmp/hardening/mfb build -q && ./build/app2.out
```

(Scratch copies: /tmp/hardening/B03/modules/neg_hiddenfield, neg_hiddenctor, neg_pubwith;
installed-.mfp variant: /tmp/hardening/B03/modules/mfp_app.)

- Observed: `Wrote executable to ./build/app2.out`. The read case prints `2`, and the WITH case prints `77`.
- Expected: `error[... TYPE_MEMBER_NOT_VISIBLE]` on each of the three lines.

Contrast cases (correct today):
- `PRIVATE closed AS Integer` on an `EXPORT TYPE`, read from the importer, gives `error: TYPE_MEMBER_NOT_VISIBLE: Field `shapes.Priv::closed` is not visible from this file.`
- The same field constructed from the importer gives `TYPE_MEMBER_NOT_VISIBLE: Constructor `shapes.Priv` cannot set hidden field `closed``
- A `PUBLIC` field used across files of the same package or project still works, as it should.

| Environment | dependency form | Result |
| --- | --- | --- |
| macOS AArch64 | `local://` source package | fails ✗ |
| macOS AArch64 | installed `packages/shapes.mfp` | fails ✗ (prints `2`) |

## Root Cause

`src/ir/verify/mod.rs:TypeEnv::build` records only fields whose visibility is
`Some("private")` into `private_fields`. `src/ir/verify/values.rs:hidden_from_here`
and the constructor check in `src/ir/verify/compat.rs` (the "cannot set hidden field"
arm) consult only that set, and compare *files*, never *packages*. So nothing models
"package-visible" fields, and an importer's file is treated like any other foreign file
that may see every non-private field.

There is a second, related defect in the `.mfp` encoding.
`src/ir/lower.rs:lower_field` stores an explicit `PUBLIC` field as `"public"`
(via `src/ir/json.rs:visibility_name`). `src/binary_repr/writer.rs:put_field_payload`
has arms only for `"private"=>1`, `"package"=>2` and `"export"=>3`, so `"public"`
falls to `_ => 0`. `src/binary_repr/reader.rs:decode_type_field` decodes `0` as
`Export`. The `"package"` arm is dead because no producer emits that string. The
compiled package therefore records a `PUBLIC` field as exported. An omitted
visibility is encoded the same way (0 → Export) whatever the containing type's
visibility. The reader's visibility is currently not consulted (`BinaryReprTypeField.visibility`
is `#[allow(dead_code)]`), so this mis-encoding is latent, but any fix that wants to
enforce package-visibility from `.mfp` metadata will read wrong data.

## Goal

- From an importing project, `.field` read, positional or named constructor, and
  `WITH` on a `PUBLIC` field of an imported record all emit `TYPE_MEMBER_NOT_VISIBLE`.
- `put_field_payload` encodes `"public"` as `2` (Public), and the omitted-visibility
  default follows the spec (EXPORT for an EXPORT type, else PUBLIC).

### Non-goals (must NOT change)

- In-package or in-project cross-file access to `PUBLIC` fields stays legal.
- `EXPORT` fields stay fully accessible to importers.
- Do not "fix" this by changing the spec to say PUBLIC fields are visible to importers.

## Blast Radius

- `src/ir/verify/values.rs:hidden_from_here` — field read; fixed by this bug
- `src/ir/verify/compat.rs` constructor visibility arm — fixed by this bug
- `src/ir/verify/values.rs` `IrValue::WithUpdate` arm — has no visibility check at all
  (see B03-with-bypasses-private-field); fix together
- `src/binary_repr/writer.rs:put_field_payload` — mis-encodes `"public"`; fixed by this bug
- `src/binary_repr/reader.rs:decode_type_field` — maps `0` to Export; revisit with the writer

## Fix Design

Carry each field's declared visibility (resolved with the spec default) plus the
declaring package identity into `TypeEnv`. Treat a field as hidden when it is `private`
and the file differs, **or** it is `public` and the current file's package differs.
Merged package types already carry an identity prefix (`shapes.Secret`) that can serve
as the package key. Encode `"public"` as 2 in the writer.

## Validation Plan

- Regression test(s): a syntax fixture with a package plus an importer, covering the three
  access forms, each expecting `TYPE_MEMBER_NOT_VISIBLE`. Add a writer unit test that
  `Some("public")` encodes to 2.
- Runtime proof: the repro above fails to build.
- Doc sync: none expected (spec already states the rule).
- Full suite: the project's acceptance suite.

## Summary

The risk is in identifying "same package" reliably for merged IR. The encoding fix is
mechanical.
