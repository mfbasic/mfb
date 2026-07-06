# MFBASIC for Python developers

MFBASIC compared with Python: EAFP with a type checker, parallelism without the GIL

## Introduction

MFBASIC shares more philosophy with Python than its BASIC surface suggests.
The error model is EAFP — call the function, handle the failure — not
look-before-you-leap. The syntax is line-oriented and keyword-heavy, built to
be read aloud. Data is plain (`TYPE` is a frozen dataclass), and behavior
lives in modules — packages of free functions — rather than deep class
hierarchies.

What changes: MFBASIC is compiled and statically typed, but inference does
the paperwork — `LET name = "world"` needs no annotation, and the checker
runs before the program does, so the `TypeError` you'd meet in production is
a build failure instead. There is no interpreter and no venv: `mfb build`
produces one native executable. There is no GIL, so threads actually run in
parallel. And there is no garbage collector — every value has one owner and
is reclaimed deterministically at scope exit, which is why the `with`
statement's guarantees apply to everything, everywhere.

## Ownership and threading

Python threads share every object but can't run bytecode in parallel (the
GIL), so real parallelism means `multiprocessing` — separate interpreters,
queues, and pickling everything across.

```python
q = multiprocessing.Queue()          # pickles each message; hope it's picklable
p = multiprocessing.Process(target=word_count, args=(q,))
```

MFBASIC keeps that architecture's honesty — workers share nothing, data moves
through queues — but in one process, with typed messages and no pickle
surprises:

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

`Thread OF String TO Integer` declares both directions of the contract:
messages in are `String`, the result is `Integer`, checked at compile time
rather than discovered as an `AttributeError` inside the worker. A sent value
is *moved* — the receiver owns it, nothing is shared, and there is no
"is it picklable?" category of bug. `waitFor` is `p.join()` plus collecting
the result in one step: the worker's return value, or — if the worker failed
— its `Error`, delivered into your error handling like any local failure.

## Error handling

You already work EAFP: try the call, catch what goes wrong. MFBASIC keeps
that shape and removes the two chronic hazards — `except` clauses that catch
too much, and failures nobody wrote a handler for surfacing three modules
away. Every call either produces its value or fails with a typed `Error`
value (`code`, `message`, `source`); a failure routes to the function's
`TRAP`, or fails the function to its caller. Propagation is automatic like
exceptions, but there is no unwinding machinery — it is ordinary, auditable
value flow.

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

The function-level `TRAP` is a single `except` block for the whole function
body, at the bottom, binding one well-known type — no
`except Exception as e` overreach, no bare `except:` swallowing a
`KeyboardInterrupt`. `err.code = errorCode::ErrPathNotFound` replaces
matching on `FileNotFoundError`; `FAIL error(...)` is
`raise RuntimeError(...) from e`, with the original location preserved in
`err.source`. The inline `TRAP`/`RECOVER` is your
`try: port = load() except: port = 8080` in expression position. And `FAIL`
is the only raise: there are no exceptions-as-control-flow, no
`StopIteration` leaking through generators.

## Data vs behavior

`TYPE` is `@dataclass(frozen=True)` as the only kind of class there is: named
typed fields, structural equality, no methods. Everything you'd write as a
method or a dunder goes in a package function instead — and since Python
already pushed you toward `len(x)` over `x.length()`, the free-function style
will feel oddly familiar.

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

`WITH inv { paid := TRUE }` is `dataclasses.replace(inv, paid=True)` — and
because values are owned, not referenced, there is no aliasing to defend
against: no `copy.deepcopy`, no mutable-default-argument trap, no caller
mutating the list you stored. Where Python 3.10's `match` narrows
`isinstance`-style over an open world, MFBASIC matches over closed unions
with exhaustiveness the checker enforces:

```
TYPE Card
  amount AS Float
END TYPE

TYPE Wire
  amount AS Float
END TYPE

UNION Payment                               ' a closed set of alternatives
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

This is `match`/`case` with class patterns, plus the guarantee `mypy`'s
`assert_never` idiom approximates: add a member to `Payment` and every
`MATCH` that misses it stops compiling — no forgotten `case _:` fallthrough
at 2 a.m.

## Deterministic resource cleanup

The `with` statement exists because `__del__` is unreliable: GC runs
whenever, so files must be closed by protocol instead. MFBASIC has no GC —
scope exit *is* the collection point — so the `with` guarantee is simply how
every binding behaves. A resource is bound with `RES`:

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

Read it as `with open(src) as input, open(dst, "w") as output:` wrapped
around the whole scope — without the indentation tax, and impossible to skip,
because `RES` is the only way to hold a file at all. Both handles close on
every exit — return, `FAIL`, propagated error — in reverse declaration
order, like nested `with` blocks unwinding. Using a closed handle is a
*compile* error, not a `ValueError: I/O operation on closed file`, and the
close nobody checks in Python (`f.close()` can fail!) is observable by
calling `fs::close(f)` explicitly.

## Checked numeric semantics

Python is honestly good here — `int` never overflows, `int("12,5")` raises,
`1/0` raises — and MFBASIC keeps that "fail, don't corrupt" ethos while using
fixed-width native integers for speed. The remaining gap is floats: in
Python, `float("nan")` and `inf` flow silently through arithmetic into your
results; a `statistics.mean` of a list with one NaN is NaN, and nothing ever
raises.

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

`Integer` is 64-bit, so unlike Python's `int` it *can* overflow — and when it
would, it fails with `ErrOverflow` (77050010) instead of wrapping, routed
through the normal error model like a raised `OverflowError`. The float
story is stricter than Python's: a NaN or infinity can never be stored,
returned, or printed — the computation fails at the boundary where the value
would become observable. `toInt`/`toByte`/`toFloat` behave like `int()` and
`float()` with `ValueError` — exact result or failure — plus range checks
Python doesn't need but 64-bit values do.

## Where to go next

- `mfb man tour` — the one-page language tour.
- `mfb man errors`, `mfb man thread`, `mfb man collections` — the models above in full.
- `mfb spec language type-inference` — how far inference goes without annotations.
