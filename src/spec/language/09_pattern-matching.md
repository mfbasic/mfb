# 9. Pattern Matching

`MATCH` binds concrete union member values and matches literals; exhaustiveness is checked at compile time. A call scrutinee auto-unwraps (use an inline `TRAP` for local error handling, §8.4).

```basic
FUNC area(s AS Shape) AS Float
  MATCH s
    CASE Circle(c) : RETURN 3.14159 * c.radius * c.radius
    CASE Rect(r)   : RETURN r.w * r.h
    CASE Point(p)  : RETURN 0.0
  END MATCH
END FUNC
```

Also handles values (replacing the old `SELECT CASE`):

```basic
MATCH grade
  CASE "A"      : io::print("Great")
  CASE "B", "C" : io::print("OK")
  CASE ELSE     : io::print("?")
END MATCH
```

- If the scrutinee type is a union type, each non-`ELSE` union case must bind one local: `CASE MemberType(binding)`.
- The scrutinee keeps its declared union type. The bound case local has the concrete member type.
- Literal patterns and comma-separated literal lists.
- `NOTHING`, `TRUE`, `FALSE`, strings, and numbers are literal patterns.
- Enum matches use qualified enum member patterns such as `Color.Red`. An enum case parses as a member-access literal, so the `Type.Member` qualifier is required for the arm to count toward exhaustiveness — a bare `CASE Red` does not.
- Guards: `CASE Rect(r) WHEN r.w = r.h : ...`.
- `CASE ELSE` is the catch-all fallback.
- **Exhaustiveness**: unions must cover all member types. Open types (`Integer`, `String`, etc.) require a `CASE ELSE` or it is a compile error. Guarded `CASE` arms do not contribute to compile-time coverage because the guard can fail; use an unguarded arm or `CASE ELSE` to cover the remaining values.
- A call scrutinee auto-unwraps to its value; to handle its failure locally, use an inline `TRAP` (see §8.4). `CASE Ok`, `CASE Error`, and `CASE Err` are not valid match arms (`TYPE_RESULT_NOT_MATCHABLE`) — a failure is never matched, only trapped.

> Implementer note: `MATCH` over a fallible resource transfer (`thread.send` / `thread.transfer`) is a special case. The compiler reuses the standard union `CASE Error(e)` arm but, on the failure path, rebinds the moved resource into the case local so it can be retried. This sits at the intersection with the threading/resource specs; the surface pattern syntax is unchanged.
