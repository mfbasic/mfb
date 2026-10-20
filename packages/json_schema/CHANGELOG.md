# Changelog

All notable changes to the `json_schema` package. The package follows
[Semantic Versioning](https://semver.org/): from 1.0.0 on, a breaking change to
the exported API or to documented validation behaviour requires a new major
version.

## 1.0.0

The first stable release. It is a redesign of the 0.1.0 validator: code written
against 0.1.0 needs the changes under *Changed* that touch it, and the rest is
additive.

### Added

- `json_schema::compileWithResources(document, resources)` compiles a schema
  that refers to other schemas the caller already has. `resources` maps URIs to
  documents; nothing is retrieved. A `$ref` may name a location in any of them.
- The Draft 2020-12 metaschemas are built in. A `$ref` to the dialect's
  metaschema, or to one of its vocabularies, resolves to the bundled copy.
- `$dynamicRef` and `$dynamicAnchor`, with the 2020-12 dynamic scope.
- `json_schema::ValidationError.schemaPath` names a location in a registered
  document as its URI and a JSON Pointer.

### Changed

- `multipleOf` is exact decimal arithmetic. Each number is read as the shortest
  decimal that reads back as its Float, so `0.3` is a multiple of `0.1`. The
  0.1.0 division in Float rejected it (`0.3 / 0.1` is `2.9999999999999996`).
- A `pattern` or `patternProperties` that the regular-expression engine cannot
  compile is refused by `compile`. 0.1.0 accepted such a schema and then raised
  from `validate`.
- A `$ref` must name a schema. 0.1.0 accepted a `$ref` into a non-schema value
  and treated it as `true`.
- A class range with a class escape at either end, such as `[\d-z]`, is refused
  as ECMA-262's `u` mode refuses it.
- `$id` and `$ref` must be URI references (RFC 3986 characters, `%` escapes
  well formed, at most one `#`). A `$id` with a space is refused.
- A `$schema` with an empty fragment (`.../2020-12/schema#`) is accepted.
- The Draft 7 keywords that 2020-12 renamed or removed are refused by name:
  `dependencies`, `additionalItems`, `definitions` and `id`. 0.1.0 ignored them,
  which silently stopped enforcing a `dependencies` constraint.
- The `Schema` fields are `PUBLIC`: only `compile` builds one.
- `$dynamicRef` is no longer refused.

### Fixed

- A `$ref` or `pattern` under a keyword the package does not read raised
  `ErrNotFound` or `ErrInvalidFormat` from `validate`. Such a region is now
  checked when a reference reaches it, and its identifiers are indexed.
- A relative root `$id` was resolved against itself, so `../x.json` from it
  resolved where it should not.
- Validation of a schema with one error per item was quadratic in the item
  count: 50,000 items took 27 seconds. It is linear now.
- `uniqueItems` compared every pair of items: 20,000 distinct integers took 14
  seconds. It is linear now, through canonical keys.
- Compiling a schema was quadratic in its size: 3,000 properties with a
  reference each took 40 seconds. It is linear now.

### Removed

- `json_schema::isAbsolute`, which nothing in the package used.
