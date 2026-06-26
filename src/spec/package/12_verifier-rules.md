# Verifier Rules

The `.mfp` verifier runs before a package can be imported or merged.

The verifier must reject malformed, unsafe, or incompatible packages before any package code runs.

Verification operates on **decoded IR**, not a flat opcode stream. The structured form is easier to verify — structure is explicit, so there is no CFG reconstruction and no "reject jumps into trap/cleanup regions." Most invariants reuse the compiler's existing IR-level passes (type checking, ownership/resource linearity, exhaustiveness, return/effect agreement) rather than a parallel flat-binary representation verifier.

Current compiler source of truth:

- Verification and package-read failures are currently surfaced as detailed package/container validation messages from the package reader and verifier implementation, not as a single emitted `rules.rs` diagnostic family.
- The spec should therefore treat the concrete rejection conditions below as normative for current behavior, with message text such as invalid magic, unsupported version, invalid signature header, truncated section table, missing section, identity mismatch, or other malformed-container diagnostics.

## Container verifier

The container verifier checks:

* Magic bytes.
* Container version.
* Binary Representation (Binary Representation) version — MFPC major must be `2`; the old flat payload (major `1`) is rejected.
* Signature type and signature length.
* Signature validity.
* Header string validity.
* Exact `binaryReprLength`.
* No trailing bytes.
* Header identity, identKey, identFingerprint, and signingFingerprint match manifest identity, identKey, identFingerprint, and signingFingerprint.

## Section verifier

The section verifier checks:

* Required sections exist.
* No duplicate required sections.
* Section offsets are in range.
* Section ranges do not overlap.
* Section payloads parse exactly.
* Unknown required sections reject the package.
* Optional unknown sections may be ignored only if their flags permit ignoring.

## Type verifier

The type verifier checks:

* All `typeId` references are valid.
* No open template declarations exist.
* Template instantiations are concrete.
* `Map` keys are comparable.
* Union member indexes are valid.
* Record field indexes are valid.
* Function types have valid parameter and return types.
* `CPtr` does not appear in ordinary MFBASIC type signatures.

## Function verifier (IR-level)

The function verifier checks the decoded Binary Representation of each function:

* Every IR node is type-correct — operands, calls, constructors, member access, and `Result` inspection (`ResultIsOk`/`ResultValue`/`ResultError`) are well-typed.
* Every binding is defined before use; no use-after-move.
* Every path through the body produces a `Result` consistent with the declared success type — declared return/effect agreement.
* The source-level rule that `PROPAGATE` appears only inside a `TRAP` region is enforced during compilation; `PROPAGATE` is lowered to a `Fail` op before serialization, so decoded IR contains no separate propagate node.
* `CallResult`/`ResultValue`/`ResultError` apply only to fallible (`Result`) expressions, on the structurally correct branch.
* `MATCH` is exhaustive (covers every value or has an `ELSE`).
* There is at most one function-level bottom `TRAP`; error routing is via the structured `Trap`/`Fail` ops, never via unwinding or arbitrary jumps.
* Calls pass the correct number and type of arguments.
* Isolated function restrictions are preserved.

Because control flow is structured (nested regions with explicit ends), there are no branch targets to validate and no "jump into a trap or cleanup region" to reject.

## Resource verifier

The resource verifier checks:

* Resource values are never copied.
* Resource values are not compared, printed, serialized, or stored in ordinary collections.
* Resource values are not captured by lambdas.
* Resource values are not sent to threads unless explicitly marked sendable.
* A resource is not used after close.
* A resource is not used after move.
* A resource is closed exactly once, by explicit close or by lexical drop at scope exit.
* A resource returned from a function transfers ownership to the caller.
* A resource passed to a consuming close function is marked closed afterward.
* A borrowed resource cannot outlive the call that borrowed it.

## Native verifier

The native verifier checks:

* All native libraries referenced by metadata are declared.
* All native symbols are declared in `NATIVE_LINK_TABLE`.
* Native wrapper function signatures match their ABI entries.
* `CString` use is explicit.
* `OUT` and `REF` lifetimes do not escape.
* `CPtr` does not escape the native boundary.
* Resource ownership is declared through `RESOURCE_TABLE`.
* A package containing native metadata sets the container native flag.

This directly addresses the `.mfp` verifier gap identified in the review: type-correct IR, define-before-use, resource ownership, structured control flow, package signature validation, and native-link manifest validation. 
