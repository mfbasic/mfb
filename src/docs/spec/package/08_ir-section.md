# Binary Representation Section

The `IR` section (id `16`) carries the structured Binary Representation payload — the faithful, versioned serialization of the project's IR functions. It replaces the retired flat `CODE` stream as the carrier of every function body.

## Payload header

```text
magic        4 bytes = "MFBR"
version      u16
IrProject    ...
```

Recommended `magic`:

```text
4D 46 42 52
M  F  B  R
```

The Binary Representation `version` is currently **`3`** (`BINARY_REPR_VERSION` in `src/ir/binary.rs`). Version 2 added per-node source locations (`loc` on `Call`/`CallResult`/`Binary`/`Unary`/`For`) and a per-function source `file`, backing read-only `Error.source`/`ErrorLoc`. Version 3 (plan-20) extends spans to the full diagnostic vocabulary: every `IrOp` (statement line), every `IrMatchCase` (case-arm line), and the declaration nodes `IrFunction`/`IrParam`/`IrType`/`IrField`/`IrVariant`/`IrBinding` (declaration line) carry a trailing `loc` (`u32` line, `u32` column; column is `1` for statement/declaration spans). These spans let the IR-level semantic checker report at the same source line the AST checker does. `decode_binary_repr` rejects any payload whose first four bytes are not `MFBR`, or whose version is not exactly `3`. (This is independent of the MFPC container version, which is major `2`.) [[src/ir/binary.rs:BINARY_REPR_VERSION]]

The payload is self-contained: integers are little-endian, strings are inline length-prefixed (`u32` byte length followed by UTF-8 bytes). Crucially, the payload does **not** reference the container's `STRING_POOL` or other interned tables — every name and type inside the `IrProject` is written inline, so a function body is fully reconstructable from this payload alone. The container's metadata sections (type table, const pool, function table, exports, ABI, …) are a **parallel, derived** view used for fast scanning, ABI compatibility, and identity checks; the consumer reconstructs executable IR from the `MFBR` payload, not from those tables. The in-memory IR is free to change behind this format; the encoding is the stable contract, and `IR → Binary Representation → IR` is an identity round-trip across every node kind.

## Payload structure (`IrProject`)

`encode_project`/`decode_project` lay the project out as: [[src/ir/binary.rs:encode_project]]

```text
name            str
entry           u8 present-flag, then if present: name str, returns str, acceptsArgs bool
bindings        vec<Binding>     (top-level LET/MUT, with initializer values)
types           vec<Type>
functions       vec<Function>    (each with its full structured body)
--- optional native LINK trailer (see native-bindings) ---
linkFunctions   vec<IrLinkFunction>
linkAliases     vec<(alias str, target str)>
```

`vec<T>` is a `u32` count followed by that many elements. The native `LINK` trailer is written only when the project has `LINK` functions or re-export aliases; a reader treats end-of-buffer after `functions` as "no trailer" (`at_end`), keeping `LINK`-free packages byte-identical to the pre-feature encoding.

## Structured control flow (no jumps)

Control flow is encoded as nested regions with explicit ends, matching IR exactly:

```text
IF      <cond-expr> THEN <ops...> ELSE <ops...> END
WHILE   <cond-expr> DO <ops...> END
DO      <ops...> UNTIL <cond-expr>
FOR     <name> = <start> TO <end> STEP <step> DO <ops...> END
FOREACH <name> IN <iterable-expr> DO <ops...> END
MATCH   <scrutinee-expr> CASE <pattern> [<guard>] <ops...> ... [ELSE <ops...>] END
TRAP    <binding> <ops...> END
```

Structured exit out of these regions is itself encoded as leaf ops rather than jumps: `ExitLoop` (`EXIT FOR/DO/WHILE`), `ContinueLoop` (`CONTINUE FOR/DO/WHILE`), and `ExitProgram` (`EXIT PROGRAM`). There are no `JMP`, `JMP_FALSE`, label, or program-counter concepts in the format. A reader walks the tree; structure is read, never reconstructed.

## Statements / ops

`IrOp` is encoded faithfully, one tag byte per kind (`encode_op`). The current tag assignment is: [[src/ir/binary.rs:encode_op]]

```text
0  = Bind            1  = Assign          2  = AssignGlobal
3  = Return          4  = Fail            5  = Eval
6  = If              7  = Match           8  = While (conditional)
9  = ForEach        10  = Trap           11  = ExitLoop
12  = ContinueLoop  13  = ExitProgram    14  = For
15  = DoUntil       16  = While (unconditional / loop forever)
17  = StateAssign    (thread resource state assignment)
```

Every op ends with its `loc` (trailing `u32` line + `u32` column — the source statement's span, format v3). Source-level `PROPAGATE` and `RECOVER` are lowered before serialization (`PROPAGATE` becomes `Fail`; `RECOVER` is lowered into ordinary ops), so they are not distinct Binary Representation ops. There are no `RESOURCE_ENTER`/`LEAVE`/`CLOSE` ops: resource lifetime is implicit (see “Resource regions” below). The internal `Result`/`Ok` forms remain implementation-only — they appear in IR and therefore in Binary Representation, but are never user-visible.

## Expressions stay nested

`IrValue` is encoded as a tree, one tag byte per kind: `Binary { op, left, right }`, `Call { target, args }`, `CallResult { … }`, `ResultIsOk` / `ResultValue` / `ResultError`, `Constructor`, `MemberAccess`, `UnionWrap` / `UnionExtract`, literals, and identifiers. There is no flattening into per-register temporaries. `CallResult` of a built-in is just an `IrValue::CallResult` node — there is no flat built-in dispatch, so the old "unknown function" emitter failure cannot occur, and an inline `TRAP` over a built-in serializes like any other expression.

As of format v3 (plan-20-B) the IR is **fully typed**: every computed value node carries its result type as a canonical type-name string — `Call`/`CallResult` (callee result / success type), `Binary`/`Unary` (operation result, written before the trailing `loc`), `MemberAccess` (member type), and `ResultValue` (extracted success type), in addition to the nodes that always carried one (`Const`, `Constructor`, `UnionWrap`/`UnionExtract`, `WithUpdate`, list/map literals, `Closure`, `Capture`, `LocalRef`, `FunctionRef`). `ResultIsOk` is implicitly `Boolean` and `ResultError` implicitly `Error`; `Local`/`Global` references resolve through the enclosing `Bind`/param/global declarations (one environment lookup, not inference). This is what makes the package-path semantic verifier complete: a decoded package body is checkable without re-running type inference.

## Tables and references

Unlike the container metadata sections, the `MFBR` payload does **not** reference declarations by index into the container's interned tables. Names and types inside IR nodes are written inline as strings (type references are canonical type-name strings such as `"List OF Integer"` or `"Result OF Out"`, the same canonical names the `TYPE_TABLE` interns). The `TYPE_TABLE`, `CONST_POOL`, and friends are derived from the same IR for scanning and ABI purposes, but the payload itself carries everything needed to rebuild the IR standalone. This is why `IR → Binary Representation → IR` round-trips without consulting any other section.

## Consumption

A consumer **decodes** each imported package's `IR` section back into IR functions, applies the package identity prefix (`<id>.package.symbol`) as a link-time rename of every definition and reference, merges the package's types/constants/globals into the project, and lowers **everything** through the single `IR → NIR → native` path. There is no separate package binary representation→native bridge: package functions get every language feature — control flow, function-level and inline `TRAP`, all built-ins, inline-`TRAP`-on-built-in — for free, because they ride the same codegen as the executable's own code.

`<id>` is a **deterministic content hash** — never a per-build random value. Concretely (`package_identity_id`): it is the first **8 bytes of SHA-256**, rendered as **16 lowercase hex characters**, hashed over the package identity (`name`, `version`, `ident`, each prefixed by its `u64` length) followed by the entire MFPC `packageBinaryRepr` (all sections), not just this inner `MFBR`/IR payload. [[src/binary_repr/reader.rs:package_identity_id]] Because it is content-addressed, the same package reached through two dependency paths produces the same `<id>` and de-duplicates to a single merged copy, while two distinct packages that happen to share a name receive different `<id>`s and stay separate instead of colliding. The prefix is applied by the *consumer* at merge time as a consistent rename of the package's definitions **and** of every reference to them (from the executable and from other packages).
