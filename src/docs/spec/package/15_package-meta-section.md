# PACKAGE_META Section

The optional `PACKAGE_META` section (id `18`) carries human-facing package
metadata that the registry displays. Today it holds exactly one field, the
`project.json` `description`. It is self-contained: values are stored inline as
length-prefixed UTF-8, independent of `STRING_POOL`, so the section can be read
without decoding any other section. It does not contribute to the ABI hash.

The compiler emits it **only when the manifest declares a non-empty
`description`**. A package without one carries no section 18 at all — not an
empty one — so such a package is byte-identical to what a compiler predating
this section produced.[[src/binary_repr/reader.rs:encode_package_meta]]

It is named `PACKAGE_META` rather than `DESCRIPTION` so that `license`,
`keywords`, and similar fields can join it later without consuming another
section id.

## Layout

```text
u32                        fieldCount
fieldCount * MetaField

MetaField:
  u16                      fieldId    (1 = description; 2.. reserved)
  u32                      byteLength
  u8[byteLength]           value      (UTF-8)
```

A field-id/length design rather than a positional record, so that a later field
is additive *within* the section, exactly as the section itself is additive
within the container. **A reader must skip an unknown `fieldId`** rather than
erroring; the length prefix is what makes that possible without knowing what the
field was.[[src/binary_repr/reader.rs:read_package_meta]][[repository/src/abi.rs:parse_package_description]]

`description` is capped at **4096 bytes** — twice the `url` header cap, and
comfortably more than a one-paragraph summary needs. The cap is enforced both
when `project.json` is validated and again when the section is read, because a
hand-built payload never passed through manifest validation. Longer prose
belongs in the `DOC` section, which already exists for
it.[[src/manifest/mod.rs:MAX_DESCRIPTION_BYTES]]

## Forward compatibility, and its limit

Adding this section did not change the container version, the `.mfp` header,
`MANIFEST` section 1, or any `abiHash`. It could not: the section table is the
designated extension point. Every reader loads the table into a map keyed by
`sectionId` and accesses sections by positive lookup — there is no match on id,
no membership test against a known set, and no "unknown section" error path — so
a reader built before this section existed parses a package carrying it and
simply never looks at it.

**That cuts both ways, and the format has no "critical section" marker.** An old
reader silently ignores section 18 rather than refusing the package or warning
that it did not understand something. For a `description` that is exactly right:
a missing description is cosmetic, and the alternative — a flag-day rebuild of
every package in existence — is far worse.

But it means **section 18 must never carry semantically load-bearing or
security-relevant data**. Anything a consumer must not miss cannot live here,
because a consumer that predates the field will miss it and report success. If
such a field is ever needed, it requires a mechanism this format does not yet
have, not another entry in this section.

Section 18 *is* covered by the payload hash and therefore by the package
signature, like every section. A description cannot be altered without
invalidating the package — which is why the registry renders this copy rather
than taking a description from the (unsigned) publish request.
