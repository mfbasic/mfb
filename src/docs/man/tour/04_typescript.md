# MFBASIC for TypeScript developers

MFBASIC compared with TypeScript: discriminated unions with teeth, numbers that are actually integers

## Introduction

A lot of MFBASIC will look like the TypeScript you already write on a good
day: plain object shapes instead of class hierarchies, discriminated unions
narrowed by matching, functions over data, immutable-by-default bindings
(`LET` is `const`). The difference is that the good day is enforced. Types
are not erased at runtime because there is no runtime layer to erase them
from — MFBASIC compiles to a native executable, and the type system's claims
(exhaustiveness, no null, no `any`-shaped escape hatch) hold in the compiled
program, not just in the editor.

Three things to recalibrate. There is no garbage collector: every value has
one owner and is reclaimed deterministically at scope exit. There is no
exception channel: errors are typed values with automatic propagation, not
`throw`/`catch (e: unknown)`. And there is no `number`: MFBASIC has a real
64-bit `Integer`, a real IEEE `Float`, and checked conversions between them.

## Ownership and threading

TypeScript's answer to shared-state concurrency is not to have it: one event
loop, and Web Workers that communicate by `postMessage`. MFBASIC agrees with
that architecture — and makes it the typed, general concurrency model.

```ts
worker.postMessage(line);                    // structured clone, untyped
worker.onmessage = (e) => { /* any */ };     // hope both sides agree
```

An MFBASIC worker is an `ISOLATED FUNC` exported from a package. It shares
nothing with its parent, and both directions of the conversation are typed in
the thread handle itself.

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

`Thread OF String TO Integer` is the `postMessage` contract you wish
`Worker` had: messages in are `String`, the result out is `Integer`, and the
compiler holds both sides to it. Sending moves the value (the structured
clone without the cloning cost or the "is a `Date` cloneable?" rules), and
these are real OS threads doing parallel work — not slices of one event loop.
`waitFor` is `await`-shaped: it blocks for the result and delivers the
worker's value or its `Error`, with no unhandled-rejection limbo.

## Error handling

In TypeScript, `throw` is invisible in every signature, and what lands in
`catch` is `unknown` — so disciplined teams hand-roll
`type Result<T, E> = { ok: true; value: T } | { ok: false; error: E }` and
thread it through every call. MFBASIC builds exactly that shape into the
language and then hides the threading: a call evaluates to its success value,
and on failure the typed `Error` (`code`, `message`, `source`) automatically
routes to the enclosing `TRAP` or fails the function to its caller.

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

Read `TRAP(err)` as a `catch` where `err` is a known record type instead of
`unknown` — no `instanceof Error` narrowing, no rethrowing things that turned
out to be strings. Branching on `err.code` replaces the error-subclass
taxonomy, `FAIL` with a new message is your wrap-and-rethrow with the
`cause` chain's origin preserved in `err.source`, and the inline
`TRAP`/`RECOVER` is the `try { … } catch { return fallback; }` expression
you've written a hundred times. Because propagation is value flow, not an
exception channel, nothing is swallowed by a forgotten `await` — there are
no promises to leave floating.

## Data vs behavior

Idiomatic TypeScript already separates shapes from functions — an
`interface` plus a module of helpers. A `TYPE` is that interface, nominal
and closed, and packages are the module:

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

`WITH inv { paid := TRUE }` is `{ ...inv, paid: true }` — except the original
is *guaranteed* untouched, because these are owned values, not references
into a shared heap; `Readonly<T>` is the default physics, not an annotation.
Unions will feel like home, minus the ceremony:

```
TYPE Card
  amount AS Float
END TYPE

TYPE Wire
  amount AS Float
END TYPE

UNION Payment                               ' discriminated union, tag managed for you
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

That is your `{ kind: "card" } | { kind: "wire" }` with the `kind` field,
the narrowing, and the `switch` + `never`-default exhaustiveness check all
supplied by the language — and still enforced after compilation, since there
is no erasure to launder an unexpected variant through `as`.

## Deterministic resource cleanup

TypeScript is only now growing this: `using` declarations and
`Symbol.dispose` retrofit deterministic cleanup onto a GC language. MFBASIC
starts there. A resource is bound with `RES`, and its close is welded to the
binding's lexical scope:

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

Both handles close on every exit path — return, `FAIL`, propagated error —
in reverse declaration order, exactly the stacking `using` gives you. The
differences: it applies to *every* value, not just disposables (there is no
GC to fall back on, and none needed); it cannot be forgotten, because `RES`
is the only way to hold a resource; and use-after-close or double-close is a
compile error, not a runtime `TypeError`. `try`/`finally` chains and
`finally`-scoped cleanup flags have no equivalent because they have no job
left.

## Checked numeric semantics

TypeScript inherits JavaScript's single `number`: every integer is a float64,
precision quietly ends at 2^53, `parseInt("12,5")` returns `12`, `(300)` fits
anywhere, and `NaN` — the result of any slip — compares false to everything
and flows through your math unnoticed. `bigint` helps but doesn't mix. In
MFBASIC the numeric tower is real and every edge is a defined failure:

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

`Integer` is a true checked 64-bit integer — overflow fails with
`ErrOverflow` (77050010) instead of drifting into float imprecision.
`Float` is IEEE binary64, but a NaN or infinity can never reach a variable,
field, or return value: the computation fails at the boundary where the
value would become observable, so `NaN` never appears in your output.
And `toInt` is `parseInt` with standards: the exact value, or a failure —
never a partial parse.

## Where to go next

- `mfb man tour` — the one-page language tour.
- `mfb man errors`, `mfb man thread`, `mfb man types` — the models above in full.
- `mfb spec language type-inference` — how far inference goes without annotations.
