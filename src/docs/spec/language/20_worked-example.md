# 20. Worked Example

```basic
IMPORT collections
IMPORT fs
IMPORT io
IMPORT strings

TYPE Vec3
  x AS Float
  y AS Float
  z AS Float
END TYPE

FUNC parseLine(line AS String) AS Vec3
  LET parts = strings::split(line, ",")
  IF len(parts) <> 3 THEN FAIL error(77050002, "expected 3 fields")

  LET x = toFloat(strings::trim(collections::get(parts, 0)))   ' auto-propagates on failure
  LET y = toFloat(strings::trim(collections::get(parts, 1)))
  LET z = toFloat(strings::trim(collections::get(parts, 2)))
  RETURN Vec3[x, y, z]
END FUNC

FUNC loadPoints(path AS String) AS List OF Vec3
  MUT pts AS List OF Vec3 = []
  RES f = fs::openFile(path)                ' auto-propagates on failure
  WHILE NOT fs::eof(f)
    LET v = parseLine(fs::readLine(f))      ' auto-propagates to TRAP below on bad input
    pts = collections::append(pts, v)      ' optimized in place for MUT
  WEND
  RETURN pts                               ' f closed by lexical drop here; pts freezes automatically

  TRAP(err)
    io::print("Load failed: " & err.message)
    RETURN []                              ' use empty list as the function result
  END TRAP
END FUNC

SUB main()
  LET pts   = loadPoints("data.csv")
  io::print("Loaded " & toString(len(pts)) & " points")
  LET total = pts |> collections::transform(_, LAMBDA(p AS Vec3) -> p.x) |> collections::sum(_)
  io::print("Sum of x: " & toString(total))
  EXIT SUB

  TRAP(err)
    io::print("Fatal: " & err.message)   ' otherwise exits with err.code
    EXIT SUB
  END TRAP
END SUB
```

Two details a compiler implementer should note from this example: a `SUB` ends
with `EXIT SUB`, not `RETURN` (a `RETURN` inside a `SUB` is rejected as
`SUB_RETURN_FORBIDDEN`); and a `LAMBDA` parameter must declare its type
(`LAMBDA(p AS Vec3) -> ...`), since lambda parameter types are not inferred from
the pipeline.

## 20.1 Union Layering Example

Layering lets one `UNION` define a larger closed domain from an existing union
with `INCLUDES`, without subtyping or open dispatch. The example below compiles
and runs as a single program: `ExtraShape` includes every member of
`Shape` and adds `Triangle`, and a `MATCH` over `ExtraShape` must cover the whole
merged set.

```basic
IMPORT io

TYPE Circle
  radius AS Float
END TYPE

TYPE Rect
  w AS Float
  h AS Float
END TYPE

UNION Shape
  Circle
  Rect
END UNION

TYPE Triangle
  base AS Float
  height AS Float
END TYPE

UNION ExtraShape INCLUDES Shape
  Triangle
END UNION

FUNC name(s AS ExtraShape) AS String
  MATCH s
    CASE Circle(c)
      RETURN "circle"

    CASE Rect(r)
      RETURN "rect"

    CASE Triangle(t)
      RETURN "triangle"
  END MATCH
END FUNC

SUB main()
  LET a AS ExtraShape = Circle[radius := 10.0]
  LET b AS ExtraShape = Triangle[base := 3.0, height := 4.0]
  io::print(name(a))
  io::print(name(b))
END SUB
```

The resulting model is `ExtraShape = Shape + ExtraShape additions`, but the type
system remains closed and explicit: `Shape` is not a base class, `ExtraShape` is
not a subtype, there is no virtual dispatch, and the included union is named
explicitly — there is no retroactive extension of a union from outside its
definition.

> **Cross-package type layering is limited.** A consumer that `IMPORT`s a package
> can name that package's exported `TYPE`/`UNION` by qualified reference (a
> `pkg::Type` annotation resolves), because the importer registers the exported
> type *names*. Their field layout is not carried into the consumer, so
> *constructing* an imported type by qualified name (`shape::Circle[radius := 10.0]`)
> fails with `TYPE_UNKNOWN_VALUE`. The single-package form above is the working
> subset.
