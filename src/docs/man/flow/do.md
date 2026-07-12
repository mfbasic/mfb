# do

DO loop forms

## Synopsis

```
DO
  ...
LOOP UNTIL condition

DO WHILE condition
  ...
LOOP
```

## Description

The accepted `DO`/`LOOP` shapes are closed to exactly two. A post-test
`DO … LOOP UNTIL condition` runs the body first, then stops when the `Boolean`
condition becomes `TRUE`. A pre-test `DO WHILE condition … LOOP` checks the
`Boolean` condition before each iteration and may run zero times. There is no
bare `DO … LOOP`, no `DO UNTIL`, and no `LOOP WHILE`.

`EXIT DO` leaves the loop and `CONTINUE DO` skips to the next iteration; both
forms use the same `DO` keyword.

## Errors

No errors.

## Examples

Post-test loop:

```
DO : work = work + 1 : LOOP UNTIL done
```

Pre-test loop:

```
DO WHILE ready : io::print("tick") : ready = FALSE : LOOP
```

## See also

- `mfb man flow while`
- `mfb man flow for`
