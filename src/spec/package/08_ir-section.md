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

The Binary Representation `version` is currently `1`. A reader rejects any payload whose magic is not `MFBR` or whose version it does not support (the package binary representation container is separately versioned at MFPC major `2`).

The payload is self-contained: integers are little-endian, strings are inline length-prefixed (`u32` byte length followed by UTF-8 bytes). The in-memory IR is free to change behind this format; the encoding is the stable contract, and `IR → Binary Representation → IR` is an identity round-trip across every node kind.

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

`IrOp` is encoded faithfully, one tag byte per kind. The kinds are `Bind`, `Assign`, `AssignGlobal`, `Return`, `Fail`, `Eval`, the structured control-flow regions above (`If`, `Match`, `While`, `For`, `DoUntil`, `ForEach`, `Trap`), and the structured exit ops `ExitLoop`, `ContinueLoop`, and `ExitProgram`. Source-level `PROPAGATE` and `RECOVER` are lowered before serialization (`PROPAGATE` becomes `Fail`; `RECOVER` is lowered into ordinary ops), so they are not distinct Binary Representation ops. There are no resource ops: resource lifetime is implicit (see “Resource regions” below). The internal `Result`/`Ok` forms remain implementation-only — they appear in IR and therefore in Binary Representation, but are never user-visible.

## Expressions stay nested

`IrValue` is encoded as a tree, one tag byte per kind: `Binary { op, left, right }`, `Call { target, args }`, `CallResult { … }`, `ResultIsOk` / `ResultValue` / `ResultError`, `Constructor`, `MemberAccess`, `UnionWrap` / `UnionExtract`, literals, and identifiers. There is no flattening into per-register temporaries. `CallResult` of a built-in is just an `IrValue::CallResult` node — there is no flat built-in dispatch, so the old "unknown function" emitter failure cannot occur, and an inline `TRAP` over a built-in serializes like any other expression.

## Tables and references

The Binary Representation rides alongside the container's interned tables (strings, types, constants, globals, imports, exports). IR nodes that reference declarations resolve against those tables. Concrete type instantiations (such as `List OF Integer` or `Result OF Out`) appear in the `TYPE_TABLE`; the Binary Representation references them.

## Consumption

A consumer **decodes** each imported package's `IR` section back into IR functions, applies the package identity prefix (`<id>.package.symbol`) as a link-time rename of every definition and reference, merges the package's types/constants/globals into the project, and lowers **everything** through the single `IR → NIR → native` path. There is no separate package binary representation→native bridge: package functions get every language feature — control flow, function-level and inline `TRAP`, all built-ins, inline-`TRAP`-on-built-in — for free, because they ride the same codegen as the executable's own code.

`<id>` is a **deterministic content hash** of the package's identity (its header `name`, `version`, and `ident`) and its binary representation payload — never a per-build random value. Because it is content-addressed, the same package reached through two dependency paths produces the same `<id>` and de-duplicates to a single merged copy, while two distinct packages that happen to share a name receive different `<id>`s and stay separate instead of colliding. The prefix is applied by the *consumer* at merge time as a consistent rename of the package's definitions **and** of every reference to them (from the executable and from other packages).
