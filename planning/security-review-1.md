# MFBASIC Security Review

## Overall verdict

**The documented implementation is not ready for untrusted packages or security-sensitive production deployment.** The language design contains several strong safety ideas, but the current architecture does not yet consistently enforce the package-signing, package-verification, ownership, isolation, and executable-hardening guarantees described by the specifications.

This review covers the language specification, standard package, `.mfp` format, compiler architecture, and native linker design.     

No compiler source tree, test suite, `.mfp` file, Mach-O executable, or ELF executable was supplied. Consequently:

* Findings about documented contradictions and declared implementation behavior are confirmed.
* Findings about emitted binary flags and actual verifier behavior are “not demonstrated” rather than proven absent.
* A final artifact certification requires inspection of real `.mfp`, Mach-O, ELF-glibc, and ELF-musl outputs.

## Strong foundations

Several design choices are positive:

* Checked integer and byte arithmetic.
* Rejection of non-finite floating-point values.
* No source-level raw pointers or arbitrary native symbol lookup.
* Linear ownership for resources.
* Structured control flow in package Binary Representation rather than arbitrary jumps.
* Full-file signature coverage in the proposed `.mfp` format.
* Explicit resource sendability and ownership transfer.
* TLS certificate and server-name validation enabled by default.
* ASCII-only identifiers in the current language version.

These are useful foundations, but many depend on a verifier and backend that are not yet demonstrated to enforce the same rules.

---

# Release-blocking findings

## C-01 — Critical: current package production is unsigned

The package specification describes signed packages, Ed25519 signatures, trust policies, and default rejection of unsigned installations. The architecture explicitly states that the current package writer emits `signatureType = 0` with zero signature length.  

**Impact:** An attacker who can replace a package file can replace its code, manifest, ABI metadata, native binding declarations, and package initializer without detection.

**Resolution options:**

* Preferred: stop supporting non-local package distribution until package signing and cryptographic verification are complete.
* Implement actual Ed25519 verification, not merely validation of the signature header shape.
* Require a trusted signing public key obtained independently of the package.
* Permit unsigned local packages only behind an explicit project policy, with the exact package digest recorded in the lockfile.
* Mark builds using unsigned dependencies as non-releasable.

## C-02 — Critical: package writer and package reader disagree on the format version

The package format says MFPC major version `2` is the structured representation and that major version `1` must be rejected. The current architecture says the writer emits binary-representation version `1.0`.  

**Impact:** This can create one or more of the following:

* Packages that the same toolchain cannot reliably consume.
* Version downgrade acceptance.
* Different interpretations of the same signed bytes.
* Old flat-format packages entering code paths intended for structured IR.
* Security checks being performed against the wrong representation.

**Resolution options:**

* Use one shared version constant in writer, reader, verifier, tests, and documentation.
* Refuse package generation if the writer’s version does not match the active verifier version.
* Add golden byte-level test vectors for every supported version.
* Remove all major-version-1 code from release builds unless it is placed in a separate, explicitly unsafe conversion tool.

## C-03 — Critical: signature acceptance is not demonstrated to include cryptographic verification

The architecture says the reader’s `validate_signature_header` accepts the Ed25519 header form. It does not state that the signature is cryptographically verified against a trusted public key. 

**Impact:** A structurally correct 64-byte signature field may be mistaken for an authenticated package.

**Resolution options:**

* Separate functions and diagnostics for header validation, cryptographic verification, and trust-policy evaluation.
* Require negative tests for changed payload bytes, changed metadata, wrong keys, truncated signatures, all-zero signatures, and untrusted but valid signatures.
* Make “signature present” and “signature trusted” distinct states.
* Do not expose a package as verified until both cryptographic and trust-policy checks succeed.

## C-04 — Critical: the implemented dependency process does not match the lockfile security model

The language specification requires `mfb.lock` to record exact package versions, sources, content hashes, representation versions, native metadata hashes, and transitive dependencies. The architecture describes version pinning in `project.json` and installed files under `packages/<name>.mfp`, but does not demonstrate hash-locked transitive resolution.  

**Impact:**

* Package substitution without changing the version string.
* Rollback to a signed but vulnerable package.
* Dependency graph changes between verification and build.
* Native-library metadata substitution.
* Same-version package equivocation.

**Resolution options:**

* Make locked builds the production default.
* Record and verify the SHA-256 digest of every exact `.mfp`.
* Lock transitive package identities and native dependencies.
* Read each package once, hash the same open file descriptor, verify it, and compile from those exact bytes.
* Adopt repository metadata with rollback, freeze, key-rotation, and compromise-recovery protections. TUF explicitly defines rollback and freeze checks rather than relying on package signatures alone. ([theupdateframework.github.io][1])

## C-05 — Critical: full package verification is specified but not demonstrated

The package specification requires decoded-IR verification for typing, define-before-use, move safety, resource linearity, exhaustive matching, result handling, native metadata, and signatures. The architecture separately notes that `target/shared/validate.rs::validate_project` is currently a no-op.  

The target validator and package verifier are not necessarily the same component, but the supplied architecture does not demonstrate a single mandatory gate proving all package invariants before merge.

**Impact:** A malicious `.mfp` may bypass guarantees normally enforced by the source type checker and introduce use-after-move, double-close, invalid type operations, malformed result handling, or backend-invalid state.

**Resolution options:**

* Build one mandatory `verify_decoded_package()` gate that runs before name exposure, merge, optimization, or native lowering.
* Reuse source compiler analyses, but run them against decoded package IR without trusting serialized flags.
* Verify every function, including unreachable and private functions.
* Treat all serialized ownership, effect, and copyability flags as claims to recompute and compare.
* Run the package parser/verifier in a restricted subprocess for defense in depth.

## C-06 — Critical: `.mfp` native metadata cannot faithfully encode the language’s `LINK` semantics

Source `LINK` declarations support:

* Named ABI-slot binding.
* Arbitrary Boolean `SUCCESS_ON`/`ERROR_ON` expressions.
* `CONST` pins.
* `RESULT` expressions.
* Multiple `OUT` slots.
* `RETURN_OUT` construction.

The package format’s native symbol entry has only `returnRuleKind` and one `i64`, while its ABI entry lacks slot names, constant pins, result-expression encoding, and a complete `RETURN_OUT` mapping.  

**Impact:** A wrapper may gate success incorrectly, read uninitialized output, map arguments to the wrong C slots, return the wrong value, or mishandle native resource ownership. These failures can cross into memory corruption.

**Resolution options:**

* Preferred: lower each native wrapper into a complete, verified wrapper IR and serialize that wrapper plus a minimal declarative ABI.
* Alternative: define a complete versioned native-expression encoding covering slot names, constants, gates, result expressions, and output construction.
* Reject package generation for every `LINK` form that cannot round-trip exactly.
* Require `source LINK → .mfp → decoded LINK` identity tests.

## C-07 — Critical: package symbol identity is inconsistent and overload-unsafe

The package format proposes a deterministic content-hash identity prefix. The linker document instead describes symbols such as:

```text
_mfb_pkg_<package>_<export>
```

 

That linker spelling does not include package identity, package version, content hash, ABI hash, or overload signature.

**Impact:**

* Two registry identities with the same package name can collide.
* Two exported overloads with the same name can collide.
* Case-insensitive platform behavior can collapse distinct names.
* A malicious package may interpose on another package’s export.

**Resolution options:**

* Generate symbols from a cryptographic package-instance ID plus declaration kind and ABI hash.
* Include overload parameter and return shapes.
* Escape all names canonically; do not concatenate raw package strings.
* Reject duplicate generated symbols before code generation.
* Keep human-readable aliases only in debug metadata.

## C-08 — Critical: imported package and native code have unrestricted ambient authority

Packages can use filesystem, network, thread, terminal, and native facilities. The language describes audit metadata and inferred permissions, but no runtime or build-time capability enforcement. Native libraries are loaded before `main`, and package initializers can run before `main`.  

**Impact:** Merely importing a dependency can enable data theft, file modification, network exfiltration, denial of service, or arbitrary in-process native execution before the application’s own error handling begins.

**Resolution options:**

* Require project-granted capabilities for filesystem, network, terminal, clock, randomness, native code, and package initialization.
* Compute effects transitively from verified IR rather than trusting package-declared permission metadata.
* Prohibit new effects in an ABI-compatible update unless project policy explicitly approves them.
* Disable package initializers by default; replace them with an explicit, fallible initialization call.
* Run native bindings in a separate broker process for high-assurance deployments.

## C-09 — Critical: recommended path containment is vulnerable to TOCTOU races

The standard package recommends `canonicalPath` plus `isWithin` before accessing user-controlled paths. That is a check-then-use sequence. `openFileNoFollow` only protects the final component, not every traversed component. 

**Impact:** An attacker who can alter symlinks or directory entries between validation and opening can redirect access outside the intended directory.

**Resolution options:**

* Add a directory-handle resource and APIs such as `openWithin`, `createWithin`, `deleteWithin`, and `renameWithin`.
* On Linux, use `openat2` with `RESOLVE_BENEATH` or `RESOLVE_IN_ROOT`, plus `RESOLVE_NO_MAGICLINKS` and an appropriate symlink policy. `O_NOFOLLOW` alone protects only the final component. ([man7.org][2])
* On macOS, walk components relative to open directory descriptors and validate each result with descriptor-based metadata.
* Never treat a prior canonical string comparison as authorization for a later filesystem operation.

## C-10 — Critical: thread “isolation” is not a security boundary

The language says worker threads have fresh package instances and do not share package state. However:

* `fs::setCurrentDirectory` changes the process current directory.
* Standard streams and OS process state remain shared.
* Native libraries generally have process-global state.
* Native libraries may create their own threads.
* Dropping a worker handle requests cancellation and detaches rather than forcing termination.

 

On POSIX systems, `chdir` changes the current directory of the calling process, affecting relative path resolution. ([man7.org][3])

**Impact:** One supposedly isolated worker can change filesystem interpretation, interfere with another worker, retain CPU indefinitely, or communicate through native/global state.

**Resolution options:**

* Rename the guarantee to “language-value isolation” unless process isolation is implemented.
* Remove or prohibit `setCurrentDirectory` from worker threads.
* Use directory-relative APIs rather than process CWD.
* Use subprocess workers for untrusted packages.
* Prohibit native bindings in `ISOLATED` functions unless they execute in a separate process.
* Add CPU, memory, message-byte, open-handle, and wall-clock limits.

---

# Language and developer-facing findings

## L-01 — High: system error codes can be forged by user code

`error(code, message)` accepts any integer, while the standard package only says third parties should avoid system ranges “by convention.” 

**Impact:** A malicious package can produce `ErrNotFound`, `ErrAccessDenied`, or another trusted system code and induce a caller to take a permissive recovery path.

**Resolution options:**

* Prevent ordinary source code from constructing system-reserved codes.
* Add a package identity or error-domain field to `Error`.
* Provide namespaced error types or an opaque `ErrorCode`.
* Require callers to match both domain and code.
* Preserve a cause chain when wrapping errors.

## L-02 — High: a single broad `Error` channel encourages unsafe catch-all recovery

Every call may fail, absence is represented as an error, and a function-level trap catches all body failures.

**Impact:** Code intended to recover from “not found” can accidentally recover from permission failure, corruption, native failure, or resource exhaustion.

**Resolution options:**

* Lint every `RECOVER` or successful function-level trap that does not discriminate by code/domain.
* Let APIs declare a closed set of expected recoverable errors.
* Provide an `isExpected`/`errorDomain` mechanism.
* Keep fatal runtime conditions such as verifier corruption outside ordinary recovery.

## L-03 — High: fallible control flow is implicit, while required visibility tooling is not evidenced

The language requires editor marking of fallible calls, propagation paths, traps, permissions, and cleanup. The implementation architecture does not demonstrate `mfb audit`, LSP security annotations, or equivalent output as a completed gate.  

**Impact:** Reviewers can miss a fallible operation hidden in an argument, pipeline, constructor, or dense colon-separated line.

**Resolution options:**

* Implement `mfb audit --locked --format json` before calling the security model complete.
* Include source locations for every propagation and cleanup edge.
* Fail CI when security-sensitive effects are introduced.
* Add a formatter/linter rule forbidding multiple fallible or permissioned operations on one physical line.

## L-04 — High: dependencies may terminate the entire process with `EXIT PROGRAM`

`EXIT PROGRAM` is valid from any call depth and cannot be trapped.

**Impact:** A package can bypass application-level recovery and cause denial of service.

**Resolution options:**

* Restrict `EXIT PROGRAM` to the root package entry function.
* Alternative: require an explicit `processExit` capability.
* Lower package use of `EXIT PROGRAM` to an ordinary error unless explicitly granted.
* Include its presence in public effect/ABI metadata.

## L-05 — High: implicit close failures are not observable in source

A resource closed during lexical drop may fail; that failure is only diagnostic metadata and cannot affect program flow. 

**Impact:** Failure to flush a security database, audit log, encrypted stream, or output file may be silently treated as success.

**Resolution options:**

* Add a `mustClose` or `mustFinalize` resource property.
* Require explicit close for writable files, TLS sessions, transactions, and other integrity-sensitive resources.
* Add compiler warnings when such resources rely on implicit drop.
* Add explicit `flush`, `sync`, and directory-sync APIs.
* Define transactional ownership if explicit close itself fails.

## L-06 — High: resource signatures conflict with the ownership rules

The language says resource parameters must be declared with `RES` and resource returns with `AS RES Type`. Standard package signatures frequently show plain `File`, `Socket`, or other resource types.  

**Impact:** Different compiler layers may disagree over whether a call borrows, consumes, or returns an ordinary value, threatening double-close and use-after-close prevention.

**Resolution options:**

* Define all built-ins using the same canonical signature representation used by user code.
* Generate standard-package declarations from one machine-readable ownership schema.
* Reject any built-in signature that would be illegal source.
* Add compile-time conformance tests for every resource-producing and resource-consuming built-in.

## L-07 — High: thread resource-transfer rules are contradictory

The main thread model says resources must use `thread::transfer` on a separate resource plane. The network section says `Socket` and `UdpSocket` move through `thread::send`. Elsewhere the standard package says resources are not valid messages.  

`thread::waitFor` also closes the handle but does not statically move the binding; later operations fail only at runtime. 

**Impact:** Implementations may disagree about queue ownership, failure rollback, handle invalidation, and cleanup.

**Resolution options:**

* Make the message plane strictly resource-free.
* Require all resources to use `transfer`/`accept`.
* Make `waitFor`, explicit close, and transfer consume the source binding statically.
* Eliminate runtime-only “closed but still syntactically live” states where possible.
* Add ownership-state tests for success, timeout, cancellation, and allocation-failure paths.

## L-08 — High: important APIs are unbounded

Examples include:

* `readAll`, `readText`, and standard input lines.
* JSON parsing and recursive nesting.
* Regex replacement output.
* String splitting and normalization.
* Collection copying.
* Thread messages and results.
* `maxBytes` values supplied by untrusted code.
* Package and source parsing depth.

**Impact:** Memory, CPU, stack, and arena exhaustion.

**Resolution options:**

* Add bounded forms for all whole-input APIs.
* Give the runtime configurable global and per-operation limits.
* Bound JSON depth, collection length, string bytes, thread message bytes, and native output size.
* Fail with a specific limit error rather than `ErrOutOfMemory`.
* Include limits in package audit output.

## L-09 — High: recursion and deep drop do not have an enforceable safe failure model

The language says recursion exhaustion should become an error rather than undefined behavior and suggests iterative drop for deep values, but does not require bounded recursion or iterative cleanup.

**Impact:** Native stack overflow, process crash, or stack-clash behavior instead of an ordinary `Error`.

**Resolution options:**

* Add runtime recursion-depth accounting.
* Use iterative traversal for drop, equality, hashing, printing, and JSON operations.
* Add stack probes for large generated frames.
* Test deeply nested unions, lists, records, traps, and callbacks.
* Do not claim stack exhaustion is recoverable until demonstrated on every target.

## L-10 — High: map collision resistance is unspecified

`Map` accepts attacker-controlled string and record keys, but the hashing strategy and collision behavior are not specified.

**Impact:** Algorithmic-complexity denial of service if hashing is predictable or poorly distributed.

**Resolution options:**

* Require a per-process keyed hash for general maps.
* Preserve stable order within an unchanged map while randomizing the hash seed across runs.
* Put a maximum collision-chain or probe limit in the implementation.
* Add adversarial collision tests.

## L-11 — High: network APIs have unsafe blocking defaults

Examples include indefinite `accept`, implementation-defined connect/TLS timeouts, and read/write operations whose persistent timeout defaults may be disabled.

**Impact:** A peer can indefinitely consume a worker or exhaust connection capacity.

**Resolution options:**

* Require explicit timeouts in exported server or network-facing functions.
* Establish finite documented defaults.
* Add cancellation integration for every runtime-managed blocking operation.
* Provide operation-level deadlines rather than only persistent socket timeouts.

## L-12 — High: JSON semantics are unsafe for security-sensitive interchange

`JsonNum` stores every number as `Float`; duplicate object-name behavior is unspecified; stringify ordering is implementation-defined. 

**Impact:**

* Large integer IDs can lose precision.
* Duplicate-name parser differentials can change authorization or signature interpretation.
* `stringify` cannot safely be used for signing or hashing.

RFC 8259 notes interoperability concerns around duplicate names and number precision. RFC 8785 requires no duplicate names and defines stricter canonicalization rules for signed JSON. ([RFC Editor][4])

**Resolution options:**

* Reject duplicate object names.
* Add separate JSON integer and decimal number variants.
* Reject integers that cannot be represented exactly when using `Float`.
* Add explicit parse limits.
* Add a canonical JSON API based on a defined standard.
* Warn that ordinary `stringify` is not suitable for signatures.

## L-13 — High: no secure randomness or cryptographic standard package is defined

The supplied standard package has no application-facing CSPRNG, hashing, HMAC, password-hashing, key-storage, or constant-time comparison API.

**Impact:** Developers may invent insecure token, nonce, password, or key-generation mechanisms.

**Resolution options:**

* Add `crypto::randomBytes` backed exclusively by the OS CSPRNG.
* Add vetted hash, MAC, AEAD, KDF, password hashing, and constant-time comparison APIs.
* Do not expose general-purpose low-level primitives without safe constructions.
* Add a secret byte-buffer type with controlled copying and zeroization.

## L-14 — Medium: secrets are retained and copied without zeroization

Strings and byte lists are ordinary immutable values. Arena-backed values may remain allocated until package-instance shutdown.

**Impact:** Passwords, tokens, and keys may remain recoverable in process memory long after logical use.

**Resolution options:**

* Add a non-copyable `SecretBytes` resource or dedicated secret value type.
* Zero memory on drop.
* Prevent formatting, serialization, implicit copying, and error inclusion.
* Avoid allocating secret data in a lifetime-wide arena.

## L-15 — Medium: error text and source metadata can disclose or inject data

Unhandled errors write their messages to stderr. Package-controlled messages can contain terminal control sequences or forged-looking log lines. Source locations and generated symbols may reveal internal filenames and package structure.

**Resolution options:**

* Escape control characters in default diagnostics.
* Provide structured machine-readable errors separately from terminal text.
* Strip or remap source paths in release builds.
* Never include secret values in automatic errors.
* Add terminal-safe and log-safe formatting helpers.

## L-16 — Medium: mixed numeric behavior is unusually error-prone

The promotion table converts mixed `Float`/`Fixed` arithmetic to `Fixed`, and `/` may produce truncated integer results depending on promoted type.

**Impact:** Size, rate, financial, timeout, and authorization calculations may silently behave differently than developers expect.

**Resolution options:**

* Require explicit conversion between `Float` and `Fixed`.
* Make fractional and truncating division syntactically distinct.
* Warn on integer `/`.
* Treat mixed `Float`/`Fixed` use as a security lint in bounds, lengths, ports, and timeout calculations.

## L-17 — High: manifest source controls are not currently enforced

The architecture says `include` and `exclude` are not applied, source roots are recursively traversed, and an unknown project kind is diagnosed but processing continues. 

**Impact:**

* Excluded or test-only files may be compiled.
* A malicious source root or symlink may reach outside the project.
* Build policy may not match actual inputs.
* Secret or experimental code may enter release artifacts.

**Resolution options:**

* Canonicalize every selected path and require project-root containment.
* Define and enforce symlink behavior.
* Deduplicate overlapping source roots.
* Apply `include` and `exclude` before parsing.
* Treat unknown `kind` and every manifest diagnostic as fatal.
* Emit the complete selected-source list into provenance.

---

# `.mfp` package findings

## P-01 — Critical: parser resource limits are incomplete

Only some top-level string-length recommendations are given. Section count, table counts, function counts, nesting depth, recursive type depth, IR node count, and total allocation budgets are not comprehensively bounded.

**Impact:** A malicious package can exhaust compiler memory, CPU, or stack before semantic verification.

**Resolution options:**

* Define normative limits for every length, count, and nesting level.
* Use checked arithmetic for every `offset + length`, count multiplication, and `u64 → usize` conversion.
* Parse incrementally rather than allocating from attacker-provided counts.
* Reject duplicate singleton sections, including optional singleton sections.
* Require reserved fields and unused flag bits to be zero.
* Fuzz the parser under memory and CPU limits.

## P-02 — Critical: the stable binary encoding is not fully specified

The package document lists IR node kinds but does not provide a complete normative tag assignment and byte encoding for every node. Several flags and payload fields are referenced without complete bit definitions. Function parameters are said to be recorded, but the shown `FUNCTION_TABLE` layout does not include the complete parameter record layout. 

**Impact:** Independent readers can interpret identical signed bytes differently.

**Resolution options:**

* Publish a complete field-by-field canonical encoding.
* Define every enum discriminant and flag bit.
* Require zero-valued reserved fields.
* Publish conformance vectors and malformed vectors.
* Require byte-for-byte re-encoding identity.
* Version every section that may evolve independently.

## P-03 — High: serialized constants can violate source-language invariants

The constant pool allows `Error` values without a complete `ErrorLoc`, while source says users cannot construct `Error` records directly. It also says non-finite float constants may merely be rejected, even though the language prohibits successful non-finite values.

**Impact:** A malicious package can forge errors or introduce NaN/infinity into backend paths that assume finite values.

**Resolution options:**

* Disallow serialized `Error` constants entirely, or encode and verify complete compiler-origin metadata.
* Reject every NaN and infinity in package constants.
* Prevent ordinary package IR from directly constructing `Error` or `ErrorLoc` except through verifier-recognized lowering forms.
* Verify canonical floating-point encodings.

## P-04 — High: internal `Result` forms need stronger encapsulation checks

`Result OF T` appears in package metadata even though it is not nameable or constructible in source.

**Impact:** A malicious package may export an internal result type, construct malformed result states, or use `ResultValue` on an error path.

**Resolution options:**

* Prohibit `Result` in exported source-visible APIs.
* Require every result extraction to be dominated by the corresponding tag check.
* Disallow user-level constructors for internal success members.
* Recompute fallibility rather than trusting function-kind metadata.

## P-05 — High: package names and identities are not shown to be safe filesystem components

The installer copies a package to `packages/<name>.mfp`. The architecture’s manifest validation primarily checks that `name` is a string; the format says identifier restrictions “should” apply rather than making them a hard container rule.

**Impact:** Depending on implementation details, names containing separators, dot components, device names, control characters, or case-colliding forms may escape the package directory or overwrite another package.

**Resolution options:**

* Restrict import names to one canonical ASCII identifier grammar.
* Reject dots, separators, control characters, reserved device names, and normalization variants.
* Use the package content identity for on-disk filenames rather than raw names.
* Perform no-follow, exclusive, atomic installation.
* Check collisions using the target filesystem’s case behavior.

## P-06 — High: installation and build verification are vulnerable to file replacement races unless performed on one opened object

The architecture describes separate install, verify, and build steps over files in the package directory.

**Impact:** A package can be changed after verification but before compilation.

**Resolution options:**

* Open with no-follow semantics.
* Read, hash, verify, decode, and compile from the same immutable byte buffer or open descriptor.
* Install through a temporary file followed by atomic rename.
* Verify the destination after rename.
* Make the package store content-addressed and read-only.

## P-07 — High: `AUDIT_INFO` is optional and cannot be a trusted permission source

The package format marks audit information optional, while the language expects audit data for effects, permissions, traps, cleanup, and native links.

**Impact:** A malicious package can omit or falsify audit metadata while still containing dangerous operations.

**Resolution options:**

* Derive audit data from verified IR.
* Treat package-provided audit data as a cache only.
* Compare cached audit data against recomputed data and reject mismatches.
* Make security-relevant audit output mandatory for signed releases.

## P-08 — High: package initializers run before `main`

The package format permits package initializer functions and says the runtime runs them in dependency order before `main`. 

**Impact:** Side effects occur before the root program can install traps, establish policy, initialize logging, or restrict capabilities.

**Resolution options:**

* Ban effectful implicit package initialization.
* Permit only constant/data initialization automatically.
* Require explicit application calls for effectful initialization.
* Include initializer effects in lock and approval policy.
* Run initializers after capability setup and under a catchable error boundary.

## P-09 — Medium: `.mfp` is an implementation-disclosure format

The package contains full structured function bodies, constants, strings, source maps, native declarations, and private functions.

**Impact:** Hardcoded credentials, proprietary algorithms, internal paths, diagnostic strings, and security assumptions are extractable.

**Resolution options:**

* State explicitly that `.mfp` provides no confidentiality or obfuscation.
* Strip debug and source-map sections by default for release packages.
* Remap source paths.
* Add build-time secret scanning.
* Never place credentials or private keys in source constants.

## P-10 — High: ABI hashing needs a complete canonical and transitive effect definition

The proposed ABI hash includes caller-visible effects, defaults, resource behavior, and type shapes, but the exact canonical byte representation and transitive effect computation are not fully defined.

**Impact:** Two compilers can generate different hashes for identical APIs or identical hashes for security-relevant behavioral changes.

**Resolution options:**

* Publish exact hash preimages and conformance vectors.
* Include transitive filesystem, network, native, process-exit, initializer, thread, and global-state effects.
* Include package identity and ABI-format version.
* Never use an ABI match as a substitute for content-hash locking.

## P-11 — High: native dependency resolution is under-specified

The format includes general library flags and version constraints, but the supplied design does not fully specify platform-specific search order, exact file hash enforcement, soname/install-name validation, symlink policy, or key trust.

**Impact:** Shared-library hijacking or binding to an unintended compatible-name library.

**Resolution options:**

* Resolve native dependencies at install/build time to exact platform artifacts.
* Record their cryptographic hashes in the lockfile.
* Forbid current-directory and project-directory search by default.
* Require signed vendored libraries.
* Match expected soname/install name and architecture.
* Expose the exact resolved path and digest in audit output.

---

# Built executable findings

## E-01 — Critical until verified: standard exploit mitigations are not documented

The custom Mach-O and ELF writers describe segments, stubs, GOT/PLT data, relocations, and ad hoc signing, but do not document all of the following:

* PIE/Mach-O `MH_PIE`.
* ELF `ET_DYN` position-independent executable behavior.
* Non-executable stack metadata.
* Strict W^X segment permissions.
* RELRO.
* Immediate binding.
* Stack canaries.
* Stack-clash probing.
* AArch64 BTI.
* Pointer authentication or equivalent return-address protection.
* Control-flow integrity.

 

**Impact:** Any memory-safety defect in runtime helpers, generated code, or native libraries becomes materially easier to exploit.

**Resolution options:**

* Define a mandatory executable-hardening profile per target.
* Make the linker fail if requested hardening cannot be emitted.
* Add post-link assertions over every program header, load command, and segment.
* Test hardening on every glibc, musl, and macOS artifact.
* Maintain a documented exception process for intentionally disabled controls.

## E-02 — High: macOS output uses only an ad hoc signature

The architecture says macOS output receives an ad hoc signature. That verifies internal code pages but does not establish a trusted publisher identity.

**Impact:** It does not provide the release identity, hardened-runtime policy, library validation, or normal distribution assurances expected for downloaded macOS software.

**Resolution options:**

* Add Developer ID signing as a release stage.
* Enable the hardened runtime and library validation.
* Notarize release artifacts and staple tickets where appropriate.
* Keep ad hoc signing only for local development.

Apple documents hardened runtime/library validation and notarization as core macOS distribution protections. ([Apple Developer][5])

## E-03 — High: Linux release artifacts have no documented signing or attestation

Package signing does not authenticate the final ELF because the output also depends on the compiler, runtime helpers, target, native libraries, and linker implementation.

**Impact:** A final binary can be replaced or produced by a compromised build environment even when inputs were authentic.

**Resolution options:**

* Sign each ELF artifact independently.
* Emit provenance binding the binary digest to source, exact compiler, target, lockfile, package hashes, native dependency hashes, and build parameters.
* Publish an SBOM for components and an attestation for build provenance.
* Verify provenance before release installation.

SLSA provenance is designed to bind an artifact to the build platform, process, and inputs, while reproducible builds provide an independently verifiable source-to-binary path. ([SLSA][6])

## E-04 — Critical: native bindings execute arbitrary unsafe code inside the process

The typed wrapper protects MFBASIC source from directly manipulating pointers, but it cannot force an arbitrary C library to:

* Avoid retaining `REF` or `OUT` pointers.
* Initialize output memory correctly.
* Avoid buffer overruns.
* Respect thread safety.
* Avoid callbacks, signals, `longjmp`, or foreign exceptions.
* Avoid process-global state.
* Avoid arbitrary filesystem or network behavior.

**Impact:** Native bindings remove the language’s memory-safety and isolation guarantees.

**Resolution options:**

* Treat every native package as equivalent to arbitrary native code execution.
* Require explicit project approval.
* Use generated C/Rust shims that copy and validate every value.
* Run risky native libraries in a separate process with a narrow protocol.
* Disallow passing temporary native pointers to APIs documented to retain them.
* Require fuzzing and sanitizers for every native wrapper.

## E-05 — High: dynamic loading occurs before application control

The language says required native libraries and symbols are loaded before `main`.

**Impact:** Startup failures cannot be handled by application traps, and native library initialization occurs before the application establishes policy or logging.

**Resolution options:**

* Resolve and validate native dependencies at install/build time wherever possible.
* Delay process loading until after a trusted startup wrapper establishes policy.
* Represent load failure as an ordinary entry-level error when technically possible.
* Ban untrusted native dependencies in privileged or service binaries.

## E-06 — High: custom linker output needs independent post-link verification

The project implements its own ELF and Mach-O writers rather than using a mature platform linker.

**Impact:** Small mistakes in segment permissions, dynamic tables, relocations, symbol indexes, code-signature coverage, or loader metadata can become security vulnerabilities.

**Resolution options:**

* Parse every emitted artifact with an independent parser.
* Compare against platform ABI requirements.
* Run `readelf`/`llvm-readobj`-equivalent validation for ELF and `otool`/`codesign`-equivalent checks for Mach-O.
* Differential-test output against artifacts produced by mature linkers.
* Fuzz the linker input plans and reparse every output before release.

## E-07 — High: arena-backed values may accumulate for the lifetime of a package instance

The language permits individual arena-backed drops to be no-ops when blocks are released only at package-instance shutdown. 

**Impact:** A long-running process can exhaust memory from temporary strings, JSON trees, collections, error messages, and thread transfers even though source scopes have ended.

**Resolution options:**

* Use scoped or generational arenas.
* Reclaim blocks when lexical ownership ends.
* Add per-package-instance memory quotas.
* Reuse buffers safely.
* Keep secrets out of long-lived arenas.
* Stress-test stable-memory behavior under long-running workloads.

## E-08 — Medium: release binaries may expose extensive symbols and source information

The linker emits symbol and string tables, and generated code contains error strings and package-derived names. Debug/source-map stripping is not described as a release default.

**Impact:** Reverse engineering and exploit development become easier, and internal source paths may be disclosed.

**Resolution options:**

* Strip nonessential symbols and local names in release builds.
* Remap source paths.
* Separate external crash-symbol files.
* Keep only the minimum dynamic symbol table required by the loader.
* Verify that debug sections are absent unless explicitly requested.

---

# Additional specification inconsistencies

These should be resolved because inconsistent specifications are themselves a security risk:

1. **Unhandled entry failure:** the language says exit code `255`; the package manifest section says exit with `error.code`.  
   **Resolution:** choose one behavior and make it part of executable ABI tests. Exit `255` with the full error code in structured stderr is less vulnerable to host exit-code truncation.

2. **`SUB` return syntax:** the language says `RETURN` is forbidden in `SUB`, but worked examples use it.
   **Resolution:** replace every such example with `EXIT SUB` and add parser conformance tests.

3. **Built-in resource type naming:** the architecture says the resolver recognizes `FileHandle`; the language and standard package use `File`. 
   **Resolution:** one canonical type ID and source name.

4. **Thread type implementation:** the architecture describes two-parameter thread types, while the language adds a resource plane.
   **Resolution:** do not advertise resource-plane threads until every compiler layer and package encoding supports them.

5. **Package consumption path:** one document says packages are decoded and merged into the single IR pipeline; other architecture/linker sections describe external NIR imports and generated package symbols.
   **Resolution:** choose one path. The verified IR-merge path is preferable because it avoids a second semantic bridge.

6. **`net::poll(List OF Socket)`:** the standard package acknowledges this overload is unreachable because resources cannot be stored in ordinary lists.
   **Resolution:** remove it or introduce an explicitly safe borrowed poll-set resource.

---

# Recommended remediation order

## Stop-release requirements

Do not permit public package distribution or untrusted package imports until all of these are complete:

1. MFPC version consistency.
2. Real package signature verification and independent trust roots.
3. Content-hash lockfile enforcement for the complete dependency graph.
4. Mandatory decoded-IR verification.
5. Faithful native-wrapper serialization.
6. Collision-safe package and overload symbol identities.
7. Capability enforcement or an explicit statement that dependencies have full process authority.
8. Secure descriptor-relative filesystem APIs.
9. A documented and tested executable-hardening profile.
10. Exact resource and thread ownership semantics.

## Before production beta

Complete:

* Parser and verifier fuzzing.
* Input, recursion, message-byte, and memory limits.
* Safe JSON semantics.
* CSPRNG and secret handling.
* Explicit close/finalization policies.
* Finite network deadlines.
* Source-root containment.
* Package name and installation hardening.
* Process isolation for untrusted/native workers.

## Release engineering baseline

Produce for every release:

* Signed `.mfp` packages.
* Signed final executables.
* Exact dependency lockfile.
* SBOM.
* SLSA-style provenance.
* Reproducibility report.
* Package-verifier report.
* Mach-O/ELF hardening report.
* Fuzzing and malformed-package corpus results.
* Native-binding inventory and approval record.

These controls align with NIST’s recommendation to integrate secure development and verification practices across the software lifecycle rather than relying on a final review alone. ([NIST Computer Security Resource Center][7])

## Final risk disposition

**Safe for:** continued design work, internal experimentation, and trusted-source prototypes with native bindings disabled.

**Not currently safe for:** public package ecosystems, hostile `.mfp` input, plugins from third parties, security-sensitive services, privileged executables, or claims of sandboxed/isolated package execution.

[1]: https://theupdateframework.github.io/specification/latest/?utm_source=chatgpt.com "The Update Framework Specification"
[2]: https://man7.org/linux/man-pages/man2/openat2.2.html?utm_source=chatgpt.com "openat2(2) - Linux manual page"
[3]: https://man7.org/linux/man-pages/man2/chdir.2.html?utm_source=chatgpt.com "chdir(2) - Linux manual page"
[4]: https://www.rfc-editor.org/info/rfc8259/?utm_source=chatgpt.com "RFC 8259: The JavaScript Object Notation (JSON) Data ..."
[5]: https://developer.apple.com/documentation/security/hardened-runtime?utm_source=chatgpt.com "Hardened Runtime | Apple Developer Documentation"
[6]: https://slsa.dev/spec/v1.0/provenance?utm_source=chatgpt.com "SLSA • Provenance"
[7]: https://csrc.nist.gov/pubs/sp/800/218/final?utm_source=chatgpt.com "Secure Software Development Framework (SSDF) Version 1.1"
