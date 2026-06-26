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

`FOR EACH` accepts `List OF T` and `Map OF K TO V` sources. A list loop binds the loop variable as `T`. A map loop binds the loop variable as `MapEntry OF K TO V`; `entry.key` has type `K` and `entry.value` has type `V`. Map loop order is implementation-defined but stable for a given unchanged map value, matching the order used by `keys` and `values`.

`EXIT FOR`, `EXIT DO`, and `EXIT WHILE` leave the innermost enclosing loop whose
kind matches the keyword and continue after that loop. `CONTINUE FOR`,
`CONTINUE DO`, and `CONTINUE WHILE` skip the rest of the current iteration of
the innermost enclosing matching loop. `FOR EACH` uses `EXIT FOR` and
`CONTINUE FOR`; both `DO ... LOOP UNTIL` and `DO WHILE ... LOOP` use `EXIT DO`
and `CONTINUE DO`. The named kind may target an outer matching loop through
inner loops of other kinds; it is a compile error when no matching loop encloses
the statement.

`EXIT SUB` leaves the enclosing `SUB` successfully with no value. `EXIT FUNC` is
always a compile error because a `FUNC` must `RETURN` a value.

`EXIT PROGRAM <integer>` terminates the process with the given exit code from
any call depth. It is not catchable by `TRAP`. Before termination, the runtime
runs lexical cleanup for all live scopes in the current call chain up to the
entry point. Constant exit codes outside the host process range are compile
errors; non-constant values follow the host convention.

There is **no `GOTO`** and **no `SELECT CASE`** (use `MATCH`). `EXIT` and
`CONTINUE` are structured, lexically scoped loop/routine exits, not arbitrary
jumps.
