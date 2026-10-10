# bug-1019: a collection temporary passed through a generic function (`idOf(squares(20.0))`) is never freed

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory leak)

Status: Open
Regression Test: (proposed) a `--debug` alloc/free test over the program below, asserting `alloc_calls == free_calls`

`LET d AS List OF Square = idOf(squares(20.0))`, with `FUNC idOf OF T(x AS T) AS T`, leaks the
argument temporary. bs18-agent-j reports the same leak through `transform(idOf(temp), f)`
(`/tmp/bs18j/tp_gen4.mfb`; not re-measured by the coordinator).

**Correct behavior:** the temporary produced by the inner call is freed once; `alloc_calls ==
free_calls`, `live_bytes 0`.

## Failing Reproduction

Measured by the coordinator on 2026-10-10 on main's code (`/tmp/main-dbg-target/release/mfb`),
`mfb build -q --debug`: `arena.0.alloc_calls 5`, `free_calls 4`, `live_bytes 144`.

```
IMPORT collections
IMPORT io


TYPE Circle
  r AS Float
END TYPE

TYPE Square
  side AS Float
END TYPE

UNION Shape
  Circle
  Square
END UNION

TYPE Holder
  items AS List OF Shape
END TYPE

FUNC describe(s AS Shape) AS String
  MATCH s
    CASE Circle(c)
      RETURN "circle " & toString(c.r)
    CASE Square(q)
      RETURN "square " & toString(q.side)
  END MATCH
END FUNC

FUNC showAll(label AS String, xs AS List OF Shape) AS Nothing
  MUT line AS String = label & ":"
  FOR EACH s IN xs
    line = line & " [" & describe(s) & "]"
  NEXT
  io::print(line)
END FUNC

FUNC showMap(label AS String, m AS Map OF String TO Shape) AS Nothing
  MUT line AS String = label & ":"
  FOR EACH k IN collections::keys(m)
    line = line & " " & k & "=[" & describe(collections::get(m, k)) & "]"
  NEXT
  io::print(line)
END FUNC

FUNC showNested(label AS String, rows AS List OF List OF Shape) AS Nothing
  MUT n AS Integer = 0
  FOR EACH row IN rows
    showAll(label & " row " & toString(n), row)
    n = n + 1
  NEXT
END FUNC

FUNC squares(first AS Float) AS List OF Square
  RETURN [Square[first], Square[first + 1.0]]
END FUNC
FUNC maybe(n AS Integer) AS List OF Square
  IF n < 0 THEN
    FAIL error(1, "neg")
  END IF
  RETURN squares(toFloat(n))
END FUNC

FUNC idOf OF T(x AS T) AS T
  RETURN x
END FUNC

FUNC main AS Integer
  LET d AS List OF Square = idOf(squares(20.0))
  io::print(toString(len(d)))
  RETURN 0
END FUNC

FUNC returnedShapes AS List OF Shape
  RETURN maybe(40)
END FUNC
```

Found by bs18-agent-j while fixing bug-989; it predates that fix.
