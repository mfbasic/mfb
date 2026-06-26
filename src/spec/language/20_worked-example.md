# 20. Worked Example

```basic
IMPORT collections
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
  LET total = pts |> collections::transform(_, LAMBDA(p) -> p.x) |> collections::sum(_)
  io::print("Sum of x: " & toString(total))
  RETURN

  TRAP(err)
    io::print("Fatal: " & err.message)   ' otherwise exits with err.code
    RETURN
  END TRAP
END SUB
```

## 20.1 Package Layering Example

Package layering lets one package define a larger closed domain from another package's union without subtyping or open dispatch.

```basic
' shape/shapes.mfb
IMPORT math

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

EXPORT FUNC area(s AS Shape) AS Float
  MATCH s
    CASE Circle(c)
      RETURN math::pi * c.radius * c.radius

    CASE Rect(r)
      RETURN r.w * r.h
  END MATCH
END FUNC
```

```basic
' extraShape/shapes.mfb
IMPORT math
IMPORT shape

TYPE Triangle
  a AS Float
  b AS Float
  c AS Float
END TYPE

UNION ExtraShape INCLUDES shape::Shape
  Triangle
END UNION

EXPORT FUNC area(s AS ExtraShape) AS Float
  MATCH s
    CASE Triangle(t)
      RETURN triangleArea(t.a, t.b, t.c)

    CASE ELSE
      FAIL error(77050004, "shape member not handled")
  END MATCH
END FUNC

PRIVATE FUNC triangleArea(a AS Float, b AS Float, c AS Float) AS Float
  LET p = (a + b + c) / 2.0
  RETURN math::sqrt(p * (p - a) * (p - b) * (p - c))
END FUNC
```

Users import the larger package when they want the larger domain:

```basic
IMPORT extraShape
IMPORT io

SUB main()
  LET s AS extraShape::ExtraShape = extraShape::Circle[radius := 10.0]
  LET t AS extraShape::ExtraShape = extraShape::Triangle[a := 3.0, b := 4.0, c := 5.0]

  io::print(toString(extraShape::area(s)))
  io::print(toString(extraShape::area(t)))
END SUB
```

The resulting model is `extraShape = shape + extraShape additions`, but the type system remains closed and explicit: `shape::Shape` is not a base class, `ExtraShape` is not a subtype, there is no virtual dispatch, and no package can retroactively add member types to another package's union.
