# variable

What a variable holds, when it can change, and the one thing that is shared

## Synopsis

```
mfb man variable
```

## Imports

`variable` is a documentation topic, not an importable package. Everything on
this page is part of the core language and needs no `IMPORT`.

## Description

MFBASIC is built so that you do not have to think about memory. There is
nothing to reserve, nothing to release, and no way to leak by forgetting. This
page is the one place the whole model is written down, so that no other page
has to explain it.

There are only two ideas, and the second one has exactly one exception:

1. **A variable holds a value, and every value is independent.** Assigning or
   passing gives a copy. Changing one name can never change another.
2. **A RES handle is the exception.** A handle is not copied — a second name is
   an *alias* for the same open thing.

Everything else follows from those two.

## LET and MUT

`LET` binds a value that never changes. `MUT` binds one that can.

```basic
IMPORT io

SUB main()
  LET greeting AS String = "hello"
  MUT count AS Integer = 0

  count = count + 1
  count = count + 1

  io::print(greeting & " " & toString(count))
END SUB
```

```
hello 2
```

`LET` is the default and the one to reach for. Use `MUT` when a name genuinely
has to take a new value over time.

## Every value is independent

Assigning copies. The two names go their separate ways:

```basic
IMPORT io

SUB main()
  MUT a AS Integer = 1
  MUT b AS Integer = a
  b = 42
  io::print("a = " & toString(a) & ", b = " & toString(b))
END SUB
```

```
a = 1, b = 42
```

Passing to a call copies too, so a call cannot reach back and change your
variable:

```basic
IMPORT io

SUB bump(n AS Integer)
  MUT local AS Integer = n
  local = local + 100
  io::print("  inside bump, local = " & toString(local))
END SUB

SUB main()
  MUT n AS Integer = 7
  bump(n)
  io::print("after bump, n = " & toString(n))
END SUB
```

```
  inside bump, local = 107
after bump, n = 7
```

This is not special treatment for numbers. It is true of records and of
collections as well — a `List` you hand to a call is the callee's own copy:

```basic
IMPORT io
IMPORT collections

SUB growList(items AS List OF Integer)
  MUT local AS List OF Integer = items
  local = collections::append(local, 999)
  io::print("  inside growList, local len = " & toString(len(local)))
END SUB

SUB main()
  MUT xs AS List OF Integer = [1, 2, 3]
  MUT ys AS List OF Integer = xs
  ys = collections::append(ys, 4)
  io::print("xs len = " & toString(len(xs)) & ", ys len = " & toString(len(ys)))

  growList(xs)
  io::print("after growList, xs len = " & toString(len(xs)))
END SUB
```

```
xs len = 3, ys len = 4
  inside growList, local len = 4
after growList, xs len = 3
```

`xs` is untouched by both. There is no way to write a function in MFBASIC that
changes a `List` its caller can see.

## Changing a record: WITH

A record is a value like any other, so it is copied on assignment. To change
one, build a new one from the old with `WITH`, naming only the fields that
differ:

```basic
IMPORT io

TYPE Point
  x AS Integer
  y AS Integer
END TYPE

SUB main()
  MUT p AS Point = Point[x := 1, y := 2]
  MUT q AS Point = p
  q = WITH q { x := 50 }
  io::print("p.x = " & toString(p.x) & ", q.x = " & toString(q.x))
END SUB
```

```
p.x = 1, q.x = 50
```

`q` started as a copy of `p`, so updating `q` left `p` alone. `WITH` is the
only way to update a record's fields.

## The exception: RES handles

An open file, socket, or audio stream is not a value you can copy — there is
one real open thing behind it. Those are bound with `RES` instead of `LET` or
`MUT`, and a second name for one is an **alias**: two names, one open thing.

Because it is an alias and not a copy, handing a handle to a call and using it
afterwards works exactly as you would hope:

```basic
IMPORT io
IMPORT fs

SUB writeLine(RES f AS fs::File, text AS String)
  fs::writeAll(f, text)
  io::print("  wrote through the handle inside writeLine")
END SUB

SUB main()
  RES a AS fs::File = fs::open("/tmp/variable-demo.txt", "write")
  writeLine(a, "first\n")
  fs::writeAll(a, "second\n")
  io::print("still wrote through it after the call returned")
  fs::close(a)

  io::print("contents: " & fs::readText("/tmp/variable-demo.txt"))
END SUB
```

```
  wrote through the handle inside writeLine
still wrote through it after the call returned
contents: first
second
```

Both writes went to the same file, and `a` was still usable after `writeLine`
returned. The call was given an alias, not a copy, and it did not take the
handle away.

The flip side of one open thing is that closing through **either** name closes
it. After `fs::close(a)`, `a` cannot be used again — and neither can any other
name for the same file. Using a closed handle is refused at compile time where
the compiler can see it, and reported as `ErrResourceClosed` where it cannot.

## Handles close themselves

You usually do not call `close` at all. A handle closes when its binding's
scope ends — on every way out, including a `RETURN` and a failure:

```basic
IMPORT io
IMPORT fs

SUB useThenReturn()
  RES f AS fs::File = fs::open("/tmp/variable-scope.txt", "write")
  fs::writeAll(f, "scope test\n")
  io::print("  handle open here; the scope is about to end")
END SUB

SUB main()
  useThenReturn()
  io::print("after the scope ended: " & fs::readText("/tmp/variable-scope.txt"))
END SUB
```

```
  handle open here; the scope is about to end
after the scope ended: scope test
```

Nothing closed the file explicitly. Call `close` only when you want the handle
released *earlier* than the end of its scope.

## What goes away, and when

A value lasts as long as the name that holds it. When the scope ends, it is
gone, and you do not do anything to make that happen — there is nothing to
release and no way to leak by forgetting. The one thing worth being deliberate
about is a `RES` handle you want closed early, which is what `close` is for.

## Where to look next

- `mfb man types` — the types a variable can hold.
- `mfb man flow` — scopes, and the ways out of one.
- `mfb spec memory` — the internal memory model, for anyone working on the
  compiler itself. Nothing on this page depends on it.
