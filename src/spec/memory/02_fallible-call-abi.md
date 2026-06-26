# Fallible-Call Result ABI

A native fallible call returns its outcome in four registers:

```text
x0  tag       0 = success, 1 = error, 2 = program exit
x1  value     success: the result value (0 for Nothing); error: the Error code
x2  message   error: pointer to the error message string
x3  source    error: pointer to the origin ErrorLoc (0 = no origin)
```

The three tags are the constants `RESULT_OK_TAG` (`0`), `RESULT_ERR_TAG` (`1`),
and `RESULT_PROGRAM_EXIT_TAG` (`2`); the four registers are `RESULT_TAG_REGISTER`
(= the return register `x0`), `RESULT_VALUE_REGISTER` (`x1`),
`RESULT_ERROR_MESSAGE_REGISTER` (`x2`), and `RESULT_ERROR_SOURCE_REGISTER` (`x3`).
The program-exit tag is checked before the success/error split: at the program
entry it routes `x1` (the exit code) to the process return register and jumps to
exit, so it is distinct from both a normal success and an error.

On success only `x0`/`x1` are meaningful. On error all of `x1` (code), `x2`
(message), and `x3` (source) are set. A fresh error stamps `x3` with an
`ErrorLoc` built from the originating expression's `(file, line, char)`; a runtime
helper error is stamped at its call site; a propagated error forwards `x3`
unchanged so the origin is preserved. A null `source` (`x3 == 0`) is a valid,
origin-less error (an OOM-degraded error built when no `ErrorLoc` could be
allocated).

In the **registers**, `x2` and `x3` are **absolute pointers**. In the in-arena
`Error`/`ErrorLoc` records, however, `message` and `source` are stored as
**block-relative offsets** (see *Native Heap Value Layouts*), with offset `0` as
the null sentinel. The two conversions bridge the forms: trapping materializes a
3-field `Error` record from `x1`/`x2`/`x3` (absolute → offset), and
`FAIL <error>` / `emit_load_error_fields` loads `x1`/`x2`/`x3` back from the
`Error` value's `code`/`message`/`source` fields (offset → absolute, mapping a
0 offset back to a null pointer).
