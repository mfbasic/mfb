# bug-907: a package whose IR enum members disagree with its TYPE_TABLE is accepted, and `toString(<enum>)` then returns another member's name

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/packages/enum-ir-type-table-mismatch-rt (proposed; needs a crafted .mfp fixture)

A `.mfp` carries each exported enum twice: in `TYPE_TABLE` (ABI-hashed, used by the
importer's front end) and as a declaration inside the `IR` (`MFBR`) payload (used for
lowering). Nothing reconciles the two. If the IR spells a member differently from the
TYPE_TABLE (for example `Red, G, Blue` in IR vs `Red, Green, Blue` in TYPE_TABLE), the
package passes every import-time check (ABI hashes are recomputed from TYPE_TABLE only),
the merged-IR verifier, and links; at run time `toString(Color.Blue)` prints **`Green`**
(and with IR `Red, Grexn, Blue` it prints `Grexn`) while `toInt` still reports ordinal 2.
Every ordinal after the disagreeing member is shifted.

The spec says package IR "remain[s] untrusted input" and the merged verifier enforces
"enum well-formedness"; an enum whose name table has more entries than ordinals is not
well-formed. Correct behavior: either the reader rejects a package whose IR type
declarations disagree with its `TYPE_TABLE` (IMPORT_PACKAGE_INVALID / VERIFY_TYPE), or the
lowering uses a single source so `toString(Color.Blue)` is always the name at ordinal 2.

References:

- `src/docs/spec/package/08_ir-section.md` — "The container's metadata sections ... are a parallel, derived view ... the consumer reconstructs executable IR from the MFBR payload".
- `src/docs/spec/package/12_verifier-rules.md` — "Merge-time semantic verification ... Declarations — ... union include/member and enum well-formedness".
- `src/docs/spec/package/03_metadata-encoding.md` — ABI hashes for exported types are computed from the TYPE_TABLE structure.
- Found by B10 IR-section fuzzing (`/tmp/hardening/B10/fuzz.py 16 2 1500`, case 197: one byte of `Red` zeroed in IR → `toString(Blue)` printed `Green`).

## Failing Reproduction

`patch_mfp.py` (in `/tmp/hardening/B10/repro/`) replaces one byte string inside one MFPC
section, re-lays the section table and re-stamps `packageBinaryHash`/`binaryReprLength`.

```
cd /tmp/hardening/B10/repro
mfb init-pkg epkg && printf 'EXPORT ENUM Color\n  Red\n  Green\n  Blue\nEND ENUM\n' > epkg/src/lib.mfb
(cd epkg && mfb build)
mfb init econs && cd econs && mfb pkg add file:///tmp/hardening/B10/repro/epkg/epkg.mfp
cat > src/main.mfb <<'EOF'
IMPORT io
IMPORT epkg
FUNC main() AS Integer
  LET b AS epkg::Color = epkg::Color.Blue
  io::print("blue=" & toString(b) & " ordinal=" & toString(toInt(b)))
  RETURN 0
END FUNC
EOF
mfb build && ./build/econs.out                      # blue=Blue ordinal=2
# IR section (id 16): str "Green" -> str "G"
python3 ../patch_mfp.py ../epkg/epkg.mfp packages/epkg.mfp 16 05000000477265656e 0100000047
mfb build && ./build/econs.out
```

- Observed: build succeeds (`uses epkg - [Unsigned]`, `Wrote executable`), then `blue=Green ordinal=2`.
  With `... 16 05000000477265656e 05000000477265786e` (IR `Grexn`): `blue=Grexn ordinal=2`.
- Expected: the build rejects the package (IR/TYPE_TABLE disagreement), or prints `blue=Blue ordinal=2`.

Contrast (measured, same harness): reordering IR members (`Blue, Green, Red`) leaves output
correct; changing an IR **record** field type is rejected by the merged verifier
(`TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH`); renaming a member the consumer references is
rejected (`TYPE_UNKNOWN_ENUM_MEMBER`).

## Root Cause

`src/codegen/engine/validation/validation.rs`: `module_tables` inserts
`(type, member) → ordinal` for every IR enum member into `enum_members`, and
`from_module_and_packages` merges the package exports (decoded from TYPE_TABLE) into the
same map. With disagreeing names the map holds `(Red,0) (G,1) (Green,1) (Blue,2)`.
`compute_enum_names` then sorts the entries by `(ordinal, name)` and keeps **all of
them** as the name list — `[Red, G, Green, Blue]` — so
`src/codegen/string/repr/builder_strings.rs:lower_enum_to_string`, which indexes that list
by ordinal, maps ordinal 2 to `Green`. `compute_enum_names` assumes exactly one name per
ordinal and never checks it; `src/ir/binary.rs:verify_package` does not compare IR types
against the TYPE_TABLE.

## Goal

- A package whose IR enum (and record/union) declarations differ from its TYPE_TABLE
  entries is rejected at read time with a package diagnostic, and/or
  `compute_enum_names` errors when two names share an ordinal.

### Non-goals (must NOT change)

- Correctly built packages (TYPE_TABLE and IR agree by construction); the TYPE_TABLE/ABI wire format.

## Blast Radius

- `compute_enum_names` / `lower_enum_to_string` — fixed by this bug
- other `enum_members` consumers (`builder_collection_layout.rs`, `builder_value_semantics.rs`, `builder_values.rs`) read ordinals by name, so they are not shifted — not observed to fail
- record field types — already reconciled by the merged verifier (contrast above)
