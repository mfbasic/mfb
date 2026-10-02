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
    pts = collections::append(pts, v)      ' updated in place for MUT
  END WHILE
  RETURN pts                               ' f closed by lexical drop here; pts freezes automatically

  TRAP(err)
    io::print("Load failed: " & err.message)
    RETURN []                              ' use empty list as the function result
  END TRAP
END FUNC

FUNC main() AS Nothing
  LET pts   = loadPoints("data.csv")
  io::print("Loaded " & toString(len(pts)) & " points")
  LET total = pts |> collections::transform(_, LAMBDA(p AS Vec3) -> p.x) |> collections::sum(_)
  io::print("Sum of x: " & toString(total))
  RETURN

  TRAP(err)
    io::print("Fatal: " & err.message)   ' an uncaught entry failure exits with code 255
    RETURN
  END TRAP
END FUNC
```

Two details a compiler implementer should note from this example: the
`AS Nothing` entry ends its normal path with a bare `RETURN` before the
function-level `TRAP`, because falling through into the `TRAP` is
`TYPE_TRAP_FALLTHROUGH`; and a `LAMBDA` parameter must declare its type
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

FUNC main() AS Nothing
  LET a AS ExtraShape = Circle[radius := 10.0]
  LET b AS ExtraShape = Triangle[base := 3.0, height := 4.0]
  io::print(name(a))
  io::print(name(b))
END FUNC
```

The resulting model is `ExtraShape = Shape + ExtraShape additions`, but the type
system remains closed and explicit: `Shape` is not a base class, `ExtraShape` is
not a subtype, there is no virtual dispatch, and the included union is named
explicitly — there is no retroactive extension of a union from outside its
definition.

Imported exported records carry their field definitions and may be constructed
by qualified name, for example `shape::Circle[radius := 10.0]`, when field
visibility permits it. Source dependencies are compiled to `.mfp` before the
consumer is built and use the same imported type interface.
[[src/cli/build/source_packages.rs:build_source_dependencies]]
[[src/ir/package.rs:qualify_package_types]]

## See Also

* ./mfb spec language error-model — the auto-propagation and `FAIL` shown in the example
* ./mfb spec language collections — the `List`/`Map` operations the example uses
* ./mfb spec language types — the record `TYPE` and conversions in the example
