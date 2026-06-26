# 10. Control Flow

```basic
FOR i = 1 TO 10 STEP 2 : io::print(toString(i)) : NEXT
FOR EACH item IN lines : io::print(item) : NEXT
FOR EACH entry IN scores : io::print(entry.key & "=" & toString(entry.value)) : NEXT
WHILE x < 10 : x = x + 1 : WEND
DO : ... : LOOP UNTIL done
DO WHILE ready : ... : LOOP

IF x > 0 THEN io::print("pos") ELSE io::print("non-pos")

IF cond THEN
  ...
ELSEIF other THEN
  ...
ELSE
  ...
END IF
```

A counted `FOR` loop variable takes the numeric type promoted across the start, end, and `STEP` operands; the result may be `Byte`, `Integer`, `Fixed`, or `Float`. Non-numeric operands are a compile error (`TYPE_FOR_REQUIRES_NUMERIC`). `STEP` is optional and defaults to `1`; a constant `STEP` of zero is a compile error (`TYPE_FOR_STEP_ZERO`).

`FOR EACH` accepts `List OF T` and `Map OF K TO V` sources (any other source type is `TYPE_FOR_EACH_REQUIRES_COLLECTION`). A list loop binds the loop variable as `T`. A map loop binds the loop variable as `MapEntry OF K TO V`; `entry.key` has type `K` and `entry.value` has type `V` (any other field access is `TYPE_UNKNOWN_FIELD`). Map loop order is implementation-defined but stable for a given unchanged map value, matching the order used by `keys` and `values`.

The accepted `DO`/`LOOP` shapes are closed to exactly two: pre-test `DO WHILE <cond> ... LOOP` and post-test `DO ... LOOP UNTIL <cond>`. There is no bare `DO ... LOOP`, no `DO UNTIL`, and no `LOOP WHILE`.

`EXIT FOR`, `EXIT DO`, and `EXIT WHILE` leave the innermost enclosing loop whose
kind matches the keyword and continue after that loop. `CONTINUE FOR`,
`CONTINUE DO`, and `CONTINUE WHILE` skip the rest of the current iteration of
the innermost enclosing matching loop. `FOR EACH` uses `EXIT FOR` and
`CONTINUE FOR`; both `DO ... LOOP UNTIL` and `DO WHILE ... LOOP` use `EXIT DO`
and `CONTINUE DO`. The named kind may target an outer matching loop through
inner loops of other kinds; it is a compile error when no matching loop encloses
the statement (`EXIT_NO_MATCHING_LOOP` for `EXIT`, `CONTINUE_NO_MATCHING_LOOP`
for `CONTINUE`).

`EXIT SUB` leaves the enclosing `SUB` successfully with no value; it is a
compile error inside a `FUNC` (`EXIT_SUB_IN_FUNC`). `EXIT FUNC` is always a
compile error (`EXIT_FUNC_FORBIDDEN`) because a `FUNC` must `RETURN` a value.

`EXIT PROGRAM <integer>` terminates the process with the given exit code from
any call depth. The integer code is required (other `EXIT` forms take no
operand) and must be `Integer`-typed (`TYPE_EXIT_PROGRAM_REQUIRES_INTEGER`).
`EXIT PROGRAM` is not catchable by `TRAP`: at each returning frame it bypasses
any handler, so it propagates past every `TRAP` to the entry point. Before
termination, the runtime runs lexical cleanup for all live scopes in the current
call chain up to the entry point. A constant exit code outside the host process
range (the implementation fixes this at `0..=255`) is a compile error
(`EXIT_PROGRAM_CODE_OUT_OF_RANGE`); non-constant values follow the host
convention.

There is **no `GOTO`** and **no `SELECT CASE`** (use `MATCH`). `EXIT` and
`CONTINUE` are structured, lexically scoped loop/routine exits, not arbitrary
jumps.
