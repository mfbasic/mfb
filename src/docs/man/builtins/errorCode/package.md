# errorCode

Named `Integer` constants for the runtime error codes a `TRAP` can match on

## Synopsis

```
IMPORT errorCode
IMPORT fs

FUNC readConfig(path AS String) AS String
  RETURN fs::readAll(path)

  TRAP(err)
    IF err.code = errorCode::ErrPathNotFound THEN
      RETURN ""
    END IF
    PROPAGATE
  END TRAP
END FUNC
```

## Description

`errorCode` is a flat set of named `Integer` constants — one per runtime error
code — and nothing else. It exports no functions and declares no types. Its whole
purpose is to let a `TRAP` handler compare `err.code` against a name instead of a
magic number: `errorCode::ErrPathNotFound` rather than `77020001`.

Each name resolves to the same `Integer` the runtime puts in `Error.code`, so a
comparison is an ordinary integer equality with no conversion and no allocation.
The constants are compile-time values; referencing one costs nothing at run time.
[[src/builtins/errorcode.rs:is_errorcode_constant]]

`errorCode` is a built-in package: `IMPORT errorCode` needs no manifest
dependency. The capitalization is part of the name — `IMPORT errorcode` is not
the same package.

The registry is **generated from the specification**, not hand-maintained here:
`build.rs` reads the Constant Registry table in
`./mfb spec diagnostics error-codes` and emits the constant table this package
serves, with a drift guard that fails the build if the two disagree. That
specification topic is the single source of truth for the Name → Integer mapping,
what each code means, and which subsystem owns each numeric range; consult it
rather than this page for the value or meaning of any individual code.
[[build.rs:generate_errorcode_table]] [[src/docs/spec/diagnostics/02_error-codes.md]]

A handler that does not need to distinguish codes should not import this package
at all — `err.message` is already a human-readable string, and `PROPAGATE`
re-raises without inspecting anything.

## Errors

`errorCode` raises no errors. It exports only constants; there is nothing to
call and nothing to fail.
