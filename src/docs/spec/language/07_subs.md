# 7. Subs

A `SUB` is **effect-only and value-less**: it produces no success value, and its
call is a statement, not an expression.

```basic
SUB logItem(x AS Integer)
  io::print(toString(x))
END SUB
```

A `SUB` may be **overloaded** on the same terms as a `FUNC` (§6): several `SUB`s
may share a name when their parameter lists differ by arity or by parameter type.

A `SUB` still has an error channel — it can `FAIL`, auto-propagate, and drop
resources on the way out — but it produces nothing on success. `EXIT SUB` is the
value-less early success exit, and fall-through to `END SUB` succeeds. `RETURN`
and `RETURN NOTHING` are compile errors in a `SUB` (`SUB_RETURN_FORBIDDEN`); [[src/syntaxcheck/checking.rs]] `RETURN` is for value-producing `FUNC` bodies, and `EXIT
SUB` outside a `SUB` is `EXIT_SUB_IN_FUNC`. A `SUB` call may not be used in value
position: `LET x = aSub()` is a compile error (the call site checks the callee's
sub kind against value-less-call permission).

For first-class function typing, a `SUB(A, B, ...)` is compatible with `FUNC(A, B, ...) AS Nothing`. The compiler records a `SUB`'s signature with return type `Nothing`, [[src/syntaxcheck/mod.rs:collect_functions]] so naming a `SUB` yields a `FUNC(...) AS Nothing` value directly. This lets effect-only callbacks work without wrapper functions:

```basic
SUB printItem(x AS Integer)
  io::print(toString(x))
END SUB

collections::forEach(nums, printItem)
```

`Nothing` remains a normal concrete unit type — it is still needed for marker
union members and for the `FUNC(...) AS Nothing` callback bridge above — but a
`SUB` body never names it. A value-less call (a `SUB`, or a fallible effect-only
built-in such as `fs::writeAll`) participates in auto-propagation and inline
`TRAP` handling like any other call; its inline `TRAP` `RECOVER` takes no operand:

```basic
fs::writeAll(f, "done") TRAP(e)
  io::print(e.message)
  RECOVER            ' value-less: the call produces no value
END TRAP
io::print("saved")
```

Value-producing callbacks still require a value-producing `FUNC`. A `SUB` is valid for APIs such as `forEach` that expect `FUNC(T) AS Nothing`; it is not valid for APIs such as `transform` that infer and collect a result value.

## See Also

* ./mfb spec language functions — `FUNC` overloading and the value-producing calls a `SUB` is contrasted with
* ./mfb spec language error-model — `FAIL`, auto-propagation, and inline `TRAP` a `SUB` participates in
* ./mfb spec language types — the `Nothing` unit type and the `FUNC(...) AS Nothing` callback bridge
