# Verifier Rules

The `.mfp` reader runs before a package can be imported or merged. This page separates three things an implementer must not conflate:

1. **Import-time checks** — what the current reader (`src/binary_repr/`, `src/main.rs`) actually enforces when it opens a `.mfp`.
2. **Compile-time guarantees** — invariants established when the package *source* was compiled, and therefore assumed (not re-derived) on import.
3. **Not yet enforced** — invariants the format anticipates but the current reader does not check.

Verification operates on **decoded IR**, not a flat opcode stream. The structured form is easier to verify — structure is explicit, so there is no CFG reconstruction and no "reject jumps into trap/cleanup regions."

Current compiler source of truth:

- Container/payload rejections are surfaced as descriptive `error: ...`/`failed to read ...` strings from the reader (invalid magic, unsupported version, invalid signature header, truncated section table, missing section, identity mismatch, trailing bytes, ABI disagreement), not as a single emitted diagnostic family.

## Import-time checks (enforced by the current reader)

### Container

* Minimum length (20-byte fixed prefix), `magic`, and **exactly**
  `containerMajor.containerMinor == 1.0` (hard: no backwards compatibility).
* Signature-header consistency: `(signatureType, signatureLength)` must be `(0, 0)` or `(1, 64)`; the declared signature length must not run past end-of-file.
* Exact `binaryReprLength` — the payload must end exactly at end-of-file (no short count, no trailing bytes).
* Trust-chain completeness: a signed package must carry `identKey`, `signingKey`, `proof`, `proofSig`, `attestation`, and `attestationSig`; an unsigned package must carry none of them.
* Header identity matches the manifest identity (`validate_container_manifest_identity`): `name`, `ident`, `version`, `identKey`, and the manifest fingerprints against the fingerprints derived from the header `identKey`/`signingKey`.

The import-time reader does **not** verify the cryptographic signature, proof, attestation, or `packageBinaryHash`; that is the package manager's build-time verification chain (below). It also does not validate the container header `binaryReprMajor`/`binaryReprMinor` fields.

### Build-time trust verification (the plan-23 chain)

Before any declared dependency is decoded, merged, or lowered, the build gate
(`verify_and_report_packages` → `classify_installed_package`,
audit-1 PKG-01 + plan-23 §3.5) classifies every installed `.mfp` and prints
`uses <name> - [Verified|Unsigned|Tampered]`. The anchors are the `identKey`
pinned in the importing project's `project.json` (never the file-embedded key)
and the registry key pinned as `server.pub` (see *package-manager key-store*).
For a signed package the chain is, in order — any failure classifies the
package **Tampered** and fails the build:

1. `header.identKey` equals the pinned ident key.
2. The attestation verifies under the pinned registry key
   (`"MFP-ATTEST-v1\0"` domain) and its `repoFingerprint`, `owner`, `ident`,
   `version`, `identFingerprint`, and `signingFingerprint` all pin this exact
   package.
3. The proof verifies under the ident key (`"MFP-PROOF-v1\0"` domain) and its
   fields pin this exact package.
4. The package signature verifies under `header.signingKey` over the signed
   prefix (`"MFP-PACKAGE-v2\0" || SHA-256(prefix)`).
5. `SHA-256(packageBinaryRepr)` equals `packageBinaryHash`.

`signatureType == 0` (Unsigned) remains allowed for local `file://`/`local:`
dependencies only; a remote unsigned dependency requires the `--unsigned`
opt-in. [[src/cli/build.rs:classify_installed_package]]

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

* `decode_binary_repr` checks the `MFBR` magic and `version == 4` (the current `BINARY_REPR_VERSION`), then structurally decodes the whole `IrProject`; truncation or invalid UTF-8 anywhere in the payload is an error. [[src/ir/binary.rs:decode_binary_repr]]

## Merge-time semantic verification (enforced before native lowering)

Reading a `.mfp` reconstructs an `IrProject`, but that IR is only lowered to native code when it is *merged* into a consuming build (`merge_packages`, `src/target/shared/nir/lower.rs`). At that point — after every imported package's IR and the importer's own IR are merged into one project, and before any code is emitted — the **complete semantic verifier** runs over the merged IR (`ir::verify_semantics`, `src/ir/verify/`). A crafted `.mfp` carries hand-serialized IR that never passed any source check, so this pass re-establishes the semantic invariants codegen would otherwise trust (audit-1 finding **PKG-02**). A failure aborts the build with a `PACKAGE_BINARY_REPRESENTATION_VERIFY_*` error; the invalid IR is never lowered. [[src/target/shared/nir/lower.rs:merge_packages]]

`ir::verify` is the **single source of truth for every semantic rule** — it is the sole rejecter of those rules on *both* the source-lowered IR and decoded package IR (plan-20). The decoded package payload is fully typed (every value node carries its result type; §"Expressions" of the IR-section page) and carries the declaration-fidelity fields (`explicit_type`, `file`) the rules need, so the checker resolves each node's type from the node itself rather than re-inferring the whole expression tree.

Those annotations are **attacker-controlled**, though, so a computed node's self-reported type is never taken on faith: each is reconciled against an independent source of truth, and a disagreement is a `PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE` rejection. A `Call`/`CallResult` node must agree with the callee's declared return type; a `MemberAccess` with the declared type of the field it reads; a `Binary`/`Unary` node with the type its operands produce (`Boolean` for comparisons and logical operators, `String` for `&`, the shared operand type for arithmetic). Without this, a `String`-returning call annotated as a record made a member read typecheck against a foreign layout, and one annotated `Integer` made `result - 5` emit an integer subtract over a string pointer. A type that genuinely cannot be derived, or an `Unknown` marker on either side, is left unchecked and never rejects. [[src/ir/verify/mod.rs:check_call_result_type]]

Concretely it enforces, on package IR:

* **Type correctness** — binary/unary operand types, call/constructor argument types, return types, assignment types, list/map element/key/value types, member access on a real record member (a `MemberAccess` on a primitive is rejected), union-wrap variant membership, and match-pattern typing.
* **Arity & shape** — call arity against the callee signature, exact constructor arity (records have no field defaults), closure-`Capture` indices within their site's slot count (a body reached by capture vectors of differing length is itself rejected — lowering never emits one — and the index bound falls back to the smallest observed count, so an ambiguous shape can never disable the check), `Map` keys comparable, non-empty and **exhaustive** `MATCH` (full enum/union coverage or an `ELSE`, not merely non-empty), bounded to `256` levels of statement nesting.
* **Resource linearity** (flow-sensitive) — use-after-move / double-close across branches (`TYPE_USE_AFTER_MOVE`, cross-branch `MaybeMoved` union; see `./mfb spec language memory-semantics` §14.9), borrow invalidation (`TYPE_RESOURCE_BORROW_INVALIDATE`), the `RES`/`STATE` ownership axis (`TYPE_RESOURCE_REQUIRES_RES`, `TYPE_RES_REQUIRES_RESOURCE`, `TYPE_STATE_INVALID`, `TYPE_UNION_STATE_FORBIDDEN`), and collection-element ownership (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`, thread/resource in a `Map` key).
* **Declarations** — literal-range fit for every numeric `Const`, union include/member and enum well-formedness, duplicate union variants, cross-file member visibility (`TYPE_MEMBER_NOT_VISIBLE`), and binding/parameter well-formedness where it survives lowering.
* **Native `LINK` ABI** — C-type escape (`NATIVE_CPTR_ESCAPE`), ABI slot/param/CONST binding, and result-marker consistency over the merged link table.

This complements the structural `verify_package` re-check (unique/non-empty function and type names) that runs per-package as the IR is decoded.

The verifier remains **sound with respect to acceptance**: it rejects only what it can *prove* is malformed and skips any node whose type it genuinely cannot reconstruct (e.g. an external `LINK` result with no in-package signature), so it accepts exactly the IR a conforming front end emits.

## Compile-time guarantees (assumed on import, not re-checked)

Only the rules about **source syntax that lowering erases** are assumed rather than re-verified — and they are assumed *vacuously*, because the constructs they govern cannot appear in a `.mfp` at all (lowering normalized them away before serialization):

* Named-argument call binding (`f(x := …)` duplicate/unknown names, post-normalization arity). Packages carry only positional argument lists.
* `EXIT FUNC` / `EXIT SUB` flavor distinctions and `SUB` return-shape rules — `EXIT FUNC` lowers to nothing, `EXIT SUB`/bare `RETURN` to `Return{None}`; the flavor is gone.
* Inline-`TRAP` boundaries, fallibility, and `RECOVER`-outside-handler — the handler is treeified into ordinary ops with no boundary marker; `PROPAGATE` outside a `TRAP` *is* still caught (`TYPE_PROPAGATE_REQUIRES_TRAP`) because it serializes as an unbound-sentinel `Fail`.
* Lambda capture-escape classification and thread channel *sendability* — front-end escape/registry properties whose conclusion (not derivation) the IR records.

Because control flow is structured (nested regions with explicit ends), there are no branch targets to validate and no "jump into a trap or cleanup region" to reject — that whole class of flat-binary verification does not exist here.

The full semantic model these rules enforce is specified in `./mfb spec language error-model` (typing, `Result`/effect agreement), `./mfb spec language resource-management` (resource linearity, drop-once, sendability), and `./mfb spec language pattern-matching` (`MATCH` exhaustiveness).

## Not yet enforced by the reader

The format anticipates these, but the current reader does **not** check them. An implementer should be aware they are gaps, not guarantees:

* Section ranges may overlap. (A duplicate `sectionId`, by contrast, **is** rejected — `duplicate MFPC section id <n>`, PKG-06.)
* At *import/read* time the reader does not re-check the decoded IR; the semantic invariants are instead re-established at *merge* time, before native lowering (see "Merge-time semantic verification" above). As of plan-20 that pass is the **complete** semantic checker — it re-derives flow-sensitive resource linearity (cross-branch use-after-move, borrow invalidation), match exhaustiveness, the full type system, literal ranges, visibility, and the `LINK` ABI. The only rules it does not re-derive are the source-syntax ones that cannot appear in a package at all (see "Compile-time guarantees" above).
* No native-binding verifier — there is no `NATIVE_LINK_TABLE` section to validate; native `LINK` metadata is carried in the IR payload trailer and validated, if at all, when that IR is merged and lowered.
* No standalone signature verification in the reader (delegated to the package manager).

## See Also

* ./mfb spec architecture frontend — the two-checker split (`syntaxcheck` vs `ir::verify`)
* ./mfb spec package ir-section — the decoded IR payload the verifier operates on (fully typed, `explicit_type`/`file` fields)
* ./mfb spec language error-model — typing, `Result`, and effect-agreement guarantees
* ./mfb spec language resource-management — resource linearity and drop-once guarantees
* ./mfb spec language pattern-matching — `MATCH` exhaustiveness
