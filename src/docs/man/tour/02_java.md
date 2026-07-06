# MFBASIC for Java developers

MFBASIC compared with Java: records without classes, checked failure without checked exceptions

## Introduction

Coming from Java, the biggest shift is subtraction: there are no classes, no
inheritance, no interfaces, and no garbage collector. A `TYPE` is close to a
Java record — named, immutable fields, no identity — and behavior lives in
package-level functions, like static methods without the enclosing class.
Recent Java has been moving this way — records, sealed interfaces, pattern
matching in `switch` — and MFBASIC reads like that subset made the whole
language.

The other shift is that values are values, not references. There is no shared
heap of objects: every value has exactly one owner and is reclaimed
deterministically when its scope exits. That one property replaces the GC,
the Java Memory Model, `try`-with-resources, and defensive copying — which is
what the five examples below walk through.

## Ownership and threading

Java threads share the heap, so every mutable object two threads can reach
drags in `synchronized`, `volatile`, `java.util.concurrent`, and the memory
model. Even with virtual threads, the sharing — and the locking — remains.

```java
synchronized (lock) {          // forget this once and it still compiles
    total += countWords(line);
}
```

MFBASIC threads are isolated: no shared statics, no shared collections, no
shared anything. A worker is an `ISOLATED FUNC` exported from a package, and
the only way in or out is a bounded, typed message queue.

```
' text_workers package
IMPORT strings
IMPORT thread

EXPORT ISOLATED FUNC wordCount(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer
  MUT total = 0
  MUT line AS String = thread::receive(w, -1)    ' block until the parent sends
  WHILE line <> "done"
    total = total + len(strings::split(line, " "))
    line = thread::receive(w, -1)
  WEND
  RETURN total
END FUNC

' main package
IMPORT text_workers
IMPORT thread

LET t = thread::start(text_workers::wordCount, "")
thread::send(t, "the quick brown fox")
thread::send(t, "jumps over the lazy dog")
thread::send(t, "done")
LET words = thread::waitFor(t)      ' 9 — the worker's result, or its Error
```

Sending moves the value into the queue; the receiver gets its own value, never
a reference into your heap. Think `BlockingQueue` plus `Future.get()`, minus
the part where both sides can still touch the same object: `waitFor` delivers
the worker's typed result, or fails with the worker's `Error` — no
`ExecutionException` unwrapping. Since no state is shared, there is nothing
to synchronize and no happens-before reasoning to do.

## Error handling

Checked exceptions had the right goal — fallibility visible in signatures —
at the price of `throws` clauses and `try`/`catch` pyramids; most codebases
retreated to unchecked exceptions, where anything can throw anywhere. MFBASIC
keeps the goal and drops the ceremony. Every call either produces its value
or fails with `Error` — one public, final error shape with `code`, `message`,
and `source`. A failure transfers control to the enclosing `TRAP`, or fails
the function to its caller. Propagation is automatic like unchecked
exceptions, but it is ordinary value flow — no stack unwinding, nothing
invisible to the reader or the audit tooling.

```
IMPORT errorCode
IMPORT fs
IMPORT strings

FUNC loadPort(path AS String) AS Integer
  LET text = fs::readText(path)             ' may fail — auto-propagates to the TRAP
  LET port = toInt(strings::trim(text))     ' may fail — same
  IF port < 1 OR port > 65535 THEN FAIL error(77050002, "port out of range")
  RETURN port

  TRAP(err)
    IF err.code = errorCode::ErrPathNotFound THEN RETURN 8080     ' missing file: use the default
    FAIL error(err.code, "config " & path & ": " & err.message)   ' anything else: add context
  END TRAP
END FUNC

' handle one call locally instead of writing a handler:
LET port = loadPort("port.conf") TRAP(e)
  RECOVER 8080
END TRAP
```

A function gets at most one `TRAP`, at the bottom — the body stays a straight
happy path instead of nesting inside `try` blocks. Where Java branches on
exception classes (`catch (NoSuchFileException e)`), MFBASIC branches on
`err.code` against semantic constants. `FAIL` with a new error is your
wrap-and-rethrow, except the original origin (`err.source` — file, line,
column) rides along without a 40-frame stack trace. The inline form replaces
the one-liner `try { … } catch (E e) { return default; }` — `RECOVER`
supplies the value and execution simply continues.

## Data vs behavior

A `TYPE` is a Java record without methods — even `toString`-style behavior
lives in package functions. There is no `this`, no `equals`/`hashCode` to
hand-maintain (comparability is structural), and no builder pattern: update
is an expression.

```
TYPE Invoice
  id    AS Integer
  total AS Float
  paid  AS Boolean
END TYPE

FUNC markPaid(inv AS Invoice) AS Invoice
  RETURN WITH inv { paid := TRUE }          ' a new value; the argument is untouched
END FUNC

FUNC totalDue(invoices AS List OF Invoice) AS Float
  MUT due = 0.0
  FOR EACH inv IN invoices
    IF NOT inv.paid THEN due = due + inv.total
  NEXT
  RETURN due
END FUNC
```

`WITH` is the "wither" Java records never got. And where you'd reach for a
sealed interface plus record patterns, MFBASIC has closed unions:

```
TYPE Card
  amount AS Float
END TYPE

TYPE Wire
  amount AS Float
END TYPE

UNION Payment                               ' sealed — every permitted member is here
  Card
  Wire
END UNION

FUNC fee(p AS Payment) AS Float
  MATCH p                                   ' compile error if a member is missed
    CASE Card(c)
      RETURN c.amount * 0.029

    CASE Wire(w)
      RETURN 0.5
  END MATCH
END FUNC
```

This is `sealed interface Payment permits Card, Wire` plus an exhaustive
`switch` with record patterns — as the only dispatch mechanism, not a recent
addition living alongside `instanceof` and virtual calls. There is no
subtyping and no open hierarchy: extending a domain means declaring a new
union that `INCLUDES` this one, in your own package, with your own functions.

## Deterministic resource cleanup

`try`-with-resources works — when everyone remembers the `try` block, and
nothing leaks past the GC's schedule. In MFBASIC, cleanup is not a statement
you opt into; it is what scopes do. A resource is bound with `RES`, and the
close is attached to the binding's lexical lifetime:

```
IMPORT fs

FUNC copyHeader(src AS String, dst AS String) AS Integer
  RES input  = fs::openFile(src)            ' owned by this scope
  RES output = fs::open(dst, "write")
  MUT copied = 0
  WHILE copied < 10 AND NOT fs::eof(input)
    fs::writeAll(output, fs::readLine(input) & "\n")   ' a failure here still closes both
    copied = copied + 1
  WEND
  RETURN copied              ' output closes, then input — reverse declaration order
END FUNC
```

Both files close on every exit — return, `FAIL`, propagated error — in
reverse declaration order, the same order `try (a; b)` guarantees. The
differences: there is no way to forget the `try`, no finalizers, no
`Cleaner`, and use-after-close or double-close is a compile error rather
than an `IOException` at runtime. Plain values get the same treatment — the
deterministic teardown Java reserves for `AutoCloseable` applies to
everything, so there are no GC pauses because there is no GC.

## Checked numeric semantics

Java int arithmetic wraps silently — `Integer.MAX_VALUE + 1` is a negative
number unless you remembered `Math.addExact`. Narrowing casts truncate
silently; `(byte) 300` is 44. Doubles go NaN and the NaN propagates through
every downstream computation until something prints it. MFBASIC defines all
of it as failure:

```
FUNC nextId(id AS Integer) AS Integer
  RETURN id + 1                 ' at the 64-bit max this FAILs — it never wraps
END FUNC

FUNC percentDone(done AS Integer, total AS Integer) AS Integer
  ' total = 0 makes the DIV produce an infinity; it is caught the moment it is
  ' observed, and percentDone fails instead of returning garbage.
  RETURN toInt(toFloat(done) DIV toFloat(total) * 100.0)
END FUNC

LET b = toByte(300)             ' fails: ErrOverflow — Byte is 0..255
LET n = toInt("12,5")           ' fails: ErrInvalidFormat — no partial parses
```

Everything is `Math.*Exact` semantics by default: overflow fails with
`ErrOverflow` (77050010) and routes through the normal error model, so a
`TRAP` can handle it like any other failure. `Float` is IEEE binary64, but a
NaN or infinity can never be stored, returned, or printed — it fails at the
boundary where it would become observable, instead of surfacing as
`NaN` in a report three services later. Conversions never truncate: they
produce the exact value or fail.

## Where to go next

- `mfb man tour` — the one-page language tour.
- `mfb man errors`, `mfb man thread`, `mfb man types` — the models above in full.
- `mfb spec language memory-semantics` — the ownership model, precisely.
