# Verifier Rules

The `.mfp` reader runs before a package can be imported or merged. This page separates three things an implementer must not conflate:

1. **Import-time checks** — what the current reader (`src/binary_repr.rs`, `src/main.rs`) actually enforces when it opens a `.mfp`.
2. **Compile-time guarantees** — invariants established when the package *source* was compiled, and therefore assumed (not re-derived) on import.
3. **Not yet enforced** — invariants the format anticipates but the current reader does not check.

Verification operates on **decoded IR**, not a flat opcode stream. The structured form is easier to verify — structure is explicit, so there is no CFG reconstruction and no "reject jumps into trap/cleanup regions."

Current compiler source of truth:

- Container/payload rejections are surfaced as descriptive `error: ...`/`failed to read ...` strings from the reader (invalid magic, unsupported version, invalid signature header, truncated section table, missing section, identity mismatch, trailing bytes, ABI disagreement), not as a single emitted diagnostic family.

## Import-time checks (enforced by the current reader)

### Container

* Minimum length (26-byte prefix), `magic`, `containerMajor == 1`.
* Signature-header consistency: `(signatureType, signatureLength)` must be `(0, 0)` or `(1, 64)`; the declared signature length must not run past end-of-file.
* Exact `binaryReprLength` — the payload must end exactly at end-of-file (no short count, no trailing bytes).
* Header identity matches the manifest identity (`validate_container_manifest_identity`): `name`, `ident`, `version`, `identKey`, `identFingerprint`, `signingFingerprint`.

The reader does **not** verify the cryptographic signature; that is the package manager's responsibility (`mfb_repository::crypto`) at install/resolve time. It also does not validate the container header `binaryReprMajor`/`binaryReprMinor` fields.

### Payload / sections

* `MFPC` magic and `bcMajor == 2` (the structured-Binary-Representation clean break; `1` is rejected as predating the format).
* The section table fits within the payload, and each section's `offset + length` stays within the payload.
* The required sections are present: `MANIFEST`, `STRING_POOL`, `TYPE_TABLE`, `CONST_POOL`, `IMPORT_TABLE`, `EXPORT_TABLE`, `FUNCTION_TABLE`, `IR`, `ABI_INDEX`. (`GLOBAL_TABLE`, `RESOURCE_TABLE`, `DOC` are optional on read.) [[src/binary_repr/reader.rs:read_binary_repr_package]]
* Each metadata table parses exactly — every `read_*` table function rejects leftover trailing bytes within its section.
* `STRING_POOL` entries are valid UTF-8.
* `EXPORT_TABLE` kinds are only `1` (func) / `2` (sub); `IMPORT_TABLE`/`ABI_INDEX` pin bytes are `0`/`1`.
* The `FUNCTION_TABLE` records zero-length code regions; a non-zero `codeLength` is rejected (`flat function code stream is no longer supported`).
* `ABI_INDEX` format version is `1`, and `validate_abi_index` holds: every `EXPORT_TABLE` callable export has a matching `ABI_INDEX` entry (same name + kind) whose `sigHash` equals the hash recomputed from the function table; and the `ABI_INDEX` dependency edges match `IMPORT_TABLE` by `(name, ident)` set, per-edge `version`/`pin`, and per-edge used-symbol list.

### IR payload

* `decode_binary_repr` checks the `MFBR` magic and `version == 2`, then structurally decodes the whole `IrProject`; truncation or invalid UTF-8 anywhere in the payload is an error. [[src/ir/binary.rs:decode_binary_repr]]

## Merge-time semantic verification (enforced before native lowering)

Reading a `.mfp` reconstructs an `IrProject`, but that IR is only lowered to native code when it is *merged* into a consuming build (`merge_packages`, `src/target/shared/nir/lower.rs`). At that point — after every imported package's IR and the importer's own IR are merged into one project, and before any code is emitted — a semantic verifier runs over the merged IR (`ir::verify_semantics`, `src/ir/verify/`). A crafted `.mfp` carries hand-serialized IR that never passed the source type checker, so this pass re-establishes the semantic invariants codegen would otherwise trust (audit-1 finding **PKG-02**). A failure aborts the build with a `PACKAGE_BINARY_REPRESENTATION_VERIFY_*` error; the type-confused IR is never lowered.

The verifier is **sound with respect to acceptance**: it rejects only what it can *prove* is malformed and skips any node whose type it cannot reconstruct with certainty, so it accepts exactly the IR the front end emits today. Within that limit it enforces:

* **Member access** resolves to a real member — a `MemberAccess` on a primitive (`Integer`/`Float`/`String`/`Boolean`/`Byte`/`Fixed`/`Nothing`) is rejected, as is one on a record type that does not declare the member (fields expanded through `includes`).
* **Closure captures** stay in range — every `Capture` index is below the captured-slot count of the `Closure` site that targets the enclosing function.
* **Call / constructor arity** matches the callee — a direct call to an internal function passes an argument count within `[required, total]` for that signature, and a record constructor supplies no more arguments than the record has fields.
* **Union wraps** name a real variant — a `UnionWrap.member_type` must be a variant of the named union (variants expanded through included unions).
* **`MATCH`** carries at least one case, bounded to `256` levels of statement nesting (mirroring the decoder's depth cap).

This complements the structural `verify_package` re-check (unique/non-empty function and type names) that runs per-package as the IR is decoded.

## Compile-time guarantees (assumed on import, not re-checked)

These were enforced by the source compiler when the package was built and are **not** re-verified by the import-time reader. An importer relies on the package having been produced by a conforming compiler:

* Every IR node is type-correct — operands, calls, constructors, member access, and `Result` inspection (`ResultIsOk`/`ResultValue`/`ResultError`) are well-typed.
* Define-before-use; no use-after-move.
* Declared return/effect agreement: every path produces a `Result` consistent with the declared success type.
* `PROPAGATE` appears only inside a `TRAP` region (it is lowered to `Fail` before serialization, so decoded IR contains no separate propagate node).
* `CallResult`/`ResultValue`/`ResultError` apply only to fallible (`Result`) expressions, on the structurally correct branch.
* `MATCH` is exhaustive (covers every value or has an `ELSE`).
* Resource linearity: resources are not copied/compared/printed/serialized/stored in ordinary collections/captured by lambdas; not sent to threads unless marked sendable; closed exactly once (explicit close or lexical drop); ownership transfers on return; borrows do not outlive the call.
* Isolated-function restrictions.
* `Map` keys are comparable; `CPtr` does not appear in ordinary MFBASIC signatures.

Because control flow is structured (nested regions with explicit ends), there are no branch targets to validate and no "jump into a trap or cleanup region" to reject — that whole class of flat-binary verification does not exist here.

These guarantees are defined and enforced by the source compiler, not restated here. Their canonical specifications are `./mfb spec language error-model` (typing, `Result`/`PROPAGATE`/effect agreement), `./mfb spec language resource-management` (resource linearity, drop-once, sendability), and `./mfb spec language pattern-matching` (`MATCH` exhaustiveness).

## Not yet enforced by the reader

The format anticipates these, but the current reader does **not** check them. An implementer should be aware they are gaps, not guarantees:

* Section ranges may overlap, and a duplicate `sectionId` silently takes the last entry rather than being rejected.
* At *import/read* time the reader does not re-syntaxcheck the decoded IR; the semantic invariants are instead re-established at *merge* time, before native lowering (see "Merge-time semantic verification" above). That pass covers member access, closure-capture bounds, call/constructor arity, union-variant membership, and non-empty `MATCH`; it does **not** yet re-derive flow-sensitive resource linearity or full `Result`/effect-agreement, which still rely on the compile-time guarantees above.
* No native-binding verifier — there is no `NATIVE_LINK_TABLE` section to validate; native `LINK` metadata is carried in the IR payload trailer and validated, if at all, when that IR is merged and lowered.
* No standalone signature verification in the reader (delegated to the package manager).

## See Also

* ./mfb spec language error-model — typing, `Result`, and effect-agreement guarantees
* ./mfb spec language resource-management — resource linearity and drop-once guarantees
* ./mfb spec language pattern-matching — `MATCH` exhaustiveness
