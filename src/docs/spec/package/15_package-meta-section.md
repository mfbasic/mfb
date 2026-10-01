# PACKAGE_META Section

The optional `PACKAGE_META` section (id `18`) carries the package description.
It is independent of `STRING_POOL` and does not contribute to ABI hashes.
The writer omits it when the description is empty. Source package manifests must
contain a `description` string, but an empty string is permitted.
[[src/binary_repr/reader.rs:encode_package_meta]] [[src/manifest/mod.rs:validate_description]]

## Layout

```text
fieldCount      u32

repeated fieldCount times:
  fieldId       u16           (1 = description; other IDs are skipped)
  byteLength    u32
  value         byte[byteLength]
```

The writer emits one field, id `1`, containing UTF-8 description bytes. The
reader checks bounds for every field, skips unknown field IDs without decoding
their contents, and validates the description as UTF-8 with a maximum length of
`4096` bytes. Repeated description fields overwrite earlier values. Neither
description reader rejects trailing bytes after the declared fields.
[[src/binary_repr/reader.rs:read_package_meta]] [[repository/src/abi.rs:parse_package_description]]

The compiler returns an empty description if the section or field is absent.
The registry returns no description for an absent section/field or an invalid
MFPC section directory; it rejects malformed contents of a present section.
The registry caps `fieldCount` at `256`; the compiler's section decoder has no
corresponding count cap. [[src/binary_repr/reader.rs:read_binary_repr_package]]
[[src/binary_repr/reader.rs:read_package_meta]] [[repository/src/abi.rs:parse_package_description]]

## Integrity and compatibility

Unknown sections and fields are ignored, so this section must contain only
optional descriptive data, not information required for execution or trust.
Its bytes are covered by the payload hash and, for signed packages, the package
signature. [[wire/src/mfpc.rs:read_section_table]] [[src/target/package_mfp/mod.rs:build_package_bytes]]

## See Also

* ./mfb spec package binary-representation — section framing
* ./mfb spec package doc-section — declaration documentation
* ./mfb spec tooling project-manifest — source manifest validation
