# MFBASIC User-Defined Resource Plan

Last updated: 2026-06-17

This document plans how to formalize user-defined `RESOURCE` declarations for
ordinary source packages while preserving the existing resource ownership model.

It complements:

- `specifications/mfbasic.md`
- `specifications/standard_package.md`
- `specifications/package_format.md`
- `specifications/architecture.md`
- `specifications/memory_layouts.md`

## 1. Goal

Allow a package to export an opaque resource type backed by package-owned
MFBASIC state:

```basic
PRIVATE TYPE MyResourceState
  value AS String
END TYPE

PRIVATE FUNC closeResource(state AS MyResourceState) AS Nothing
  ' release package-owned state
END FUNC

EXPORT RESOURCE MyResource
  REPRESENTATION MyResourceState
  CLOSE closeResource
END RESOURCE
```

Importers should see `MyResource` as an ordinary concrete resource type:

- non-copyable
- ownership-movable under the existing move rules
- lexically closed on every scope exit path
- explicitly closeable through its declared close operation
- not inspectable or constructible outside the declaring package
- not thread-sendable unless explicitly declared

The design must not introduce a generic resource supertype, structural resource
matching, user-visible lifetimes, borrow annotations, or a new movability axis.

## 2. Current State

The language already defines resource semantics:

- `RESOURCE` values are unique handles with exactly one live owner.
- resource handles are not copyable.
- resource handles move by ownership transfer.
- resource handles are closed automatically by lexical drop.
- explicit close operations consume the handle.
- resource operations borrow or consume according to compiler-known metadata.
- resources cannot be stored in ordinary collections, printed, compared,
  serialized, or captured by ordinary closures.
- resources are not thread-sendable by default.
- concrete resource types may opt in to thread sendability.

The standard library already exposes opaque resource handles such as `File`,
`Socket`, `UdpSocket`, `Listener`, and `TlsSocket`.

Native `LINK` blocks already support resource declarations through:

```basic
LINK "sqlite3" AS sqlite
  TYPE Db AS RESOURCE
    CLOSE close
  END TYPE
END LINK
```

The missing feature is a source-level way for ordinary packages to declare an
exported resource whose hidden representation is package-owned MFBASIC data
rather than an opaque native pointer or compiler-owned built-in handle.

## 3. Non-Goals

This plan does not add:

- copyable resources
- non-movable resources
- a `MOVABLE` resource option
- generic `RESOURCE` parameters
- structural matching between resource types
- public field access on resources
- direct construction of resources by importers
- storing resources in ordinary `List` or `Map` values
- resource capture by ordinary closures
- user-authored `BORROW`, `MOVE`, or lifetime annotations
- finalizers for ordinary `TYPE` declarations

Resources remain nominal, concrete handle types. A resource is not an opaque
record with a destructor attached; it is a compiler-recognized owned handle
with package-private representation and declared cleanup behavior.

## 4. Proposed Syntax

Add a top-level source declaration:

```basic
visibility RESOURCE ResourceName
  REPRESENTATION RepresentationType
  CLOSE closeFunction
  THREAD_SENDABLE TRUE
END RESOURCE
```

`visibility` follows the existing top-level visibility rules:

- `PRIVATE` by default
- `PACKAGE`
- `EXPORT`

`THREAD_SENDABLE` is optional and defaults to `FALSE`:

```basic
EXPORT RESOURCE MyResource
  REPRESENTATION MyResourceState
  CLOSE closeResource
END RESOURCE
```

is equivalent to:

```basic
EXPORT RESOURCE MyResource
  REPRESENTATION MyResourceState
  CLOSE closeResource
  THREAD_SENDABLE FALSE
END RESOURCE
```

No `MOVABLE` option is defined. Resource movability is inherited from the
existing ownership model: resources are non-copyable owned values that move when
ownership transfers.

## 5. Representation Rules

`REPRESENTATION T` names the hidden storage type for the resource.

Rules:

- `T` must be a concrete type visible to the resource declaration.
- `T` must be declared in the same package as the resource.
- `T` must not be exported when used as the direct representation of an
  exported resource unless all constructors and fields needed to forge the
  representation are hidden from importers.
- importers cannot name or access the representation through the resource API.
- field access, construction, `WITH`, comparison, printing, serialization, and
  ordinary collection storage are checked against the public resource type, not
  the representation.

The representation may be a normal record that contains package-owned data. It
may contain other owned values according to the existing memory rules. If the
representation transitively contains another resource, the compiler must still
prove that cleanup closes each owned resource exactly once.

For v1, the conservative rule is:

- a user-defined resource representation must not transitively contain the same
  resource type
- recursive representation cycles are rejected unless they pass through an
  already-valid indirection form under the normal record and union recursion
  rules

## 6. Close Operation Rules

`CLOSE closeFunction` names the package-local implementation used by lexical
drop and explicit close.

The close function must:

- be visible to the resource declaration
- be declared in the same package as the resource
- accept exactly one parameter of the representation type
- return `Nothing`
- participate in normal `Result` behavior, so close may fail

Example:

```basic
PRIVATE FUNC closeResource(state AS MyResourceState) AS Nothing
  RETURN NOTHING
END FUNC
```

When a `MyResource` handle is dropped, the compiler consumes the handle,
extracts its representation, and calls `closeResource(state)`.

When close runs as lexical drop, a close failure cannot alter source-level
control flow. It is retained as cleanup diagnostic/audit metadata, matching the
existing resource rule.

When close is called explicitly through the exported close operation, close
failure is observable through ordinary `Result` propagation.

## 7. Public Close Operation

The resource declaration must expose a resource-typed close operation to code
that can name the resource type.

Recommended source spelling:

```basic
EXPORT RESOURCE MyResource
  REPRESENTATION MyResourceState
  CLOSE closeResource AS close
END RESOURCE
```

This means:

- `closeResource` is the representation-level cleanup implementation.
- `close` is the public consuming close operation for `MyResource`.
- importers call `package::close(r)` when `close` is exported.

If `AS publicName` is omitted, the compiler may either:

- synthesize an exported close operation named `close` when the resource is
  exported, or
- require an explicit `AS publicName` for exported resources.

The stricter v1 recommendation is to require `AS publicName` for exported
resources, to avoid name collisions and implicit public API additions:

```basic
EXPORT RESOURCE MyResource
  REPRESENTATION MyResourceState
  CLOSE closeResource AS closeMyResource
END RESOURCE
```

The public close operation has effective signature:

```basic
FUNC closeMyResource(r AS MyResource) AS Nothing
```

It consumes `r`. After explicit close, the source binding is moved and is not
closed again by lexical drop.

## 8. Construction Rules

Only code in the declaring package can construct a user-defined resource.

The compiler should provide a package-local wrapping form. Recommended spelling:

```basic
RETURN RESOURCE MyResource[state]
```

where `state AS MyResourceState`.

Rules:

- constructing `MyResource` consumes the representation value
- after construction, the representation is owned by the resource handle
- importers cannot use the wrapping form for exported resources
- importers cannot construct the representation and coerce it into the resource
- there is no implicit conversion between `MyResourceState` and `MyResource`

Example:

```basic
EXPORT FUNC openResource(value AS String) AS MyResource
  LET state = MyResourceState[value := value]
  RETURN RESOURCE MyResource[state]
END FUNC
```

The returned resource moves to the caller. The callee does not close it after
return because ownership has left the callee.

## 9. Borrow And Consume Behavior

Functions that accept a resource parameter continue to use the existing
compiler-known resource metadata:

- ordinary operations borrow the handle for the duration of the call
- the declared close operation consumes the handle

No source-level `BORROW` or `MOVE` parameter annotations are added.

For user-defined resources, the compiler determines consuming operations from
the resource declaration:

- the public close operation consumes
- compiler-generated lexical drop consumes
- returning a resource moves ownership to the caller
- passing a resource to a thread send operation moves only when the resource is
  thread-sendable and the send succeeds
- all other functions that accept the resource borrow it unless future metadata
  explicitly adds consuming resource operations beyond close

## 10. Thread Sendability

Resources are not thread-sendable by default.

```basic
EXPORT RESOURCE MyResource
  REPRESENTATION MyResourceState
  CLOSE closeResource AS closeMyResource
END RESOURCE
```

is not valid as `thread::send` data, thread start input, worker message, or
worker result when the resource would cross a thread boundary.

A resource may opt in:

```basic
EXPORT RESOURCE MyResource
  REPRESENTATION MyResourceState
  CLOSE closeResource AS closeMyResource
  THREAD_SENDABLE TRUE
END RESOURCE
```

The compiler must reject `THREAD_SENDABLE TRUE` unless the representation is
safe to transfer across package instances and thread arenas.

Minimum v1 rule:

- every transitive field of the representation must be thread-sendable
- any contained resource must also be thread-sendable
- function and closure values are rejected
- top-level package state must not be captured by the representation in a way
  that creates shared mutable state across package instances

Successful thread send of a user-defined resource moves ownership to the
destination side. Failed send leaves ownership with the sender. Queued
resources are owned by the runtime queue and are closed exactly once if never
received.

## 11. Package Metadata

`.mfp` packages must preserve resource metadata for imported user-defined
resources.

`RESOURCE_TABLE` already records:

- resource type
- close function
- flags
- close error behavior
- cleanup metadata

User-defined resources should use the same table with a new flag or resource
kind that distinguishes them from native and standard resources.

Current flags include:

- native resource
- standard resource
- sendable to thread

Add one resource kind representation without changing ownership semantics:

- source resource

The ABI hash for exported user-defined resources must include:

- resource type name
- representation ABI identity, without exposing hidden fields to importers
- close function signature
- public close operation name and signature
- ownership/borrow/consume behavior
- thread-sendability flag
- whether close may fail

Importers must not need the private representation layout for ordinary
typechecking, but package merging, bytecode verification, native lowering, and
audit may need enough metadata to verify close and transfer behavior.

## 12. Bytecode And Verification

The existing bytecode resource instructions remain the right model:

- `RESOURCE_ENTER`
- `RESOURCE_LEAVE`
- `CLOSE_RESOURCE`

The verifier must additionally understand source-defined resource types.

Checks:

- resource registers cannot be copied
- user-defined resources cannot be treated as their representation type
- representation values cannot escape as public resources without the resource
  construction operation
- resource values are closed, moved, or returned exactly once
- explicit close consumes the resource
- lexical drop closes any still-owned resource on every exit path
- borrowed resource handles cannot outlive the call
- non-sendable resources cannot cross thread boundaries
- sendable resources move on successful thread transfer
- failed thread transfer preserves sender ownership

The bytecode format must not expose private representation fields as importer
constructors or public field access.

## 13. Diagnostics

Add diagnostics for:

- duplicate resource declaration name
- missing representation type
- representation type not visible to resource declaration
- exported resource with forgeable exported representation
- missing close function
- close function declared in another package
- close function wrong arity
- close function parameter type mismatch
- close function return type not `Nothing`
- exported resource missing explicit public close name, if v1 requires
  `CLOSE ... AS ...`
- public close name conflicts with an existing declaration
- thread-sendable resource with non-sendable representation
- attempted construction outside declaring package
- attempted field access on a resource
- attempted `WITH` update on a resource
- attempted copy of a resource
- attempted ordinary collection storage of a resource
- attempted thread send of non-sendable resource
- use after move or close
- control-flow path that can lose or double-close a resource

If any new error codes are added, update:

- `specifications/error_codes.md`
- `specifications/mfbasic.md`
- `specifications/standard_package.md`

in the same implementation change.

## 14. Audit Behavior

`mfb audit` must report user-defined resources the same way it reports standard
and native resources.

For each resource type:

- declaring package
- source location when available
- whether the resource is standard, native, or source-defined
- public resource type name
- close operation
- whether close can fail
- thread-sendability
- cleanup regions and all exits that trigger lexical drop
- whether secondary close failures are retained as cleanup metadata

Audit output must not reveal private representation field values. It may report
private type names when source is available and the report is for the declaring
package; imported package reports should use compiled resource metadata as
authoritative.

## 15. Implementation Phases

### Phase 1: Specify Source Syntax

- Add `RESOURCE ... END RESOURCE` to the parser grammar.
- Add AST nodes for source-defined resource declarations.
- Parse `REPRESENTATION`, `CLOSE`, optional `AS publicName`, and optional
  `THREAD_SENDABLE TRUE/FALSE`.
- Reject unsupported options such as `MOVABLE`.

### Phase 2: Resolve And Typecheck

- Resolve resource names as top-level declarations.
- Resolve representation and close function references.
- Enforce representation visibility and same-package rules.
- Enforce close signature rules.
- Mark resource types as non-copyable, ownership-movable, resource handles.
- Mark thread-sendability according to declaration metadata.

### Phase 3: Construction And Close Lowering

- Add package-local resource construction lowering.
- Lower public explicit close operations to consuming close calls.
- Lower lexical drop to existing resource cleanup bytecode.
- Preserve close failure metadata for implicit cleanup.

### Phase 4: Package Metadata

- Extend package writer and reader for source-defined resources.
- Include resource metadata in `RESOURCE_TABLE`.
- Include resource behavior in ABI hash inputs.
- Ensure importers reconstruct resource ownership and sendability.

### Phase 5: Verification

- Extend bytecode verifier to validate source-defined resources.
- Reject representation/resource confusion.
- Validate cleanup regions and close function signatures.
- Validate thread boundary transfer rules.

### Phase 6: Runtime Validation

- Add tests that compile and execute programs proving:
  - lexical drop runs on normal scope exit
  - lexical drop runs on auto-propagated error exit
  - explicit close consumes the handle
  - returned resources transfer ownership to caller
  - non-sendable user resources are rejected at thread boundaries
  - sendable user resources transfer through thread queues when declared

## 16. Test Requirements

When implemented, add acceptance tests under both valid and invalid function
test directories for every created or modified function involved in resource
construction, close, import, and thread transfer.

Required valid scenarios:

- package-local construction of an exported resource
- importer receives and uses exported resource through exported functions
- explicit close succeeds and prevents lexical double-close
- lexical drop closes on success return
- lexical drop closes on error propagation
- resource returned from function is not closed by callee
- `THREAD_SENDABLE TRUE` resource moves through `thread::send`

Required invalid scenarios:

- importer attempts to construct resource directly
- importer attempts field access on resource
- resource copied then used twice
- resource stored in `List`
- close function has wrong arity
- close function has wrong parameter type
- close function returns non-`Nothing`
- `THREAD_SENDABLE TRUE` with non-sendable representation
- `THREAD_SENDABLE FALSE` resource sent to a thread
- resource used after explicit close
- resource lost or double-closed on a control-flow path

After compiler work, run:

```text
scripts/test-accept.sh target/debug/mfb target/accept-actual
```

Runtime features must also be validated by executing generated programs and
observing close behavior through exit code, stdout/stderr, file output, or
another concrete side effect.

## 17. Open Questions

1. Should exported resources require `CLOSE closeImpl AS publicClose`, or should
   the compiler synthesize a default exported `close` operation?
2. Should package-local code be able to unwrap a resource back into its
   representation, or should only the close lowering be allowed to extract it?
3. Should `REPRESENTATION` allow primitive types directly, or require a named
   package-local record for ABI stability and audit clarity?
4. How should source-defined resources be represented in native lowering when
   their representation is heap-backed or arena-backed?
5. Should the package format add an explicit `source resource` flag, or derive
   source-defined resources from the absence of `native resource` and
   `standard resource` flags?

## 18. Recommendation

Adopt source-defined resources as a first-class top-level declaration:

```basic
EXPORT RESOURCE MyResource
  REPRESENTATION MyResourceState
  CLOSE closeResource AS closeMyResource
  THREAD_SENDABLE FALSE
END RESOURCE
```

Keep the ownership model unchanged:

- resources are never copyable
- resources are ownership-movable
- resources are lexically closed
- explicit close consumes
- thread sendability is the only source-level opt-in capability

This gives ordinary packages the same resource behavior as standard and native
resources without adding a new kind of movement, lifetime, or destructor model.
