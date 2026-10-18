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
- A member that is a template instantiation is named by its template: over an `Opt OF Integer` scrutinee whose union carries `Some OF Integer`, the arm is `CASE Some(s)` and `s` has type `Some OF Integer` (§4.3).
- Literal patterns and comma-separated literal lists.
- `NOTHING`, `TRUE`, `FALSE`, strings, and numbers are literal patterns.
- An unsuffixed numeric literal pattern takes a `Fixed` or `Money` scrutinee's type, exactly as it would in a `LET` of that type, so `CASE 1.5` matches `1.5F` and `CASE 5` matches `5m`. The arm compares values of the scrutinee's type. [[src/ir/lower.rs:lower_match_case]] A literal or record arm matches exactly when the scrutinee `=` the pattern, so a `Float` arm uses IEEE equality: `CASE 0.0` matches `-0.0`. [[src/codegen/memory/value/builder_value_semantics.rs:lower_match_compare]]
- Enum matches use qualified enum member patterns such as `Color.Red`. An enum case parses as a member-access literal, so the `Type.Member` qualifier is required for the arm to count toward exhaustiveness — a bare `CASE Red` does not. The qualifier alone classifies the arm: an enum member that shares its name with a declared `TYPE`, a union member type, or another enum's member is still that enum's member, and over a union scrutinee a qualified enum arm is a pattern of the wrong type (`TYPE_MATCH_PATTERN_MISMATCH`), never the same-named union member [[src/ir/verify/matching.rs:check_match_patterns]].
- Guards: `CASE Rect(r) WHEN r.w = r.h : ...`.
- `CASE ELSE` is the catch-all fallback.
- **Exhaustiveness**: unions must cover all member types; enums must cover all members (each matched via the qualified `Type.Member` form). Open types (`Integer`, `String`, etc.) require a `CASE ELSE` or it is a compile error (`TYPE_MATCH_NOT_EXHAUSTIVE`). Guarded `CASE` arms do not contribute to compile-time coverage because the guard can fail; use an unguarded arm or `CASE ELSE` to cover the remaining values. Coverage is computed only from unguarded arms: a union arm contributes its member type name, an enum arm contributes `Type::Member`, and an unguarded `CASE ELSE` makes the match exhaustive.
- A call scrutinee auto-unwraps to its value; to handle its failure locally, use an inline `TRAP` (see §8.4). `CASE Ok`, `CASE Error`, and `CASE Err` are not valid match arms (`TYPE_RESULT_NOT_MATCHABLE`) — a failure is never matched, only trapped. (Internally, a `Result`-typed scrutinee suppresses the secondary `TYPE_MATCH_NOT_EXHAUSTIVE` cascade, since its arms already reported `TYPE_RESULT_NOT_MATCHABLE`.)

A failed `thread::transfer` is handled by `TRAP`, not `CASE Error`. Its inline handler may reuse the resource because a failed transfer retains sender ownership (§16).

## See Also

* ./mfb spec language types — the union and enum types `MATCH` binds and checks for exhaustiveness
* ./mfb spec language error-model — why a `Result` is trapped, never matched
* ./mfb spec package verifier-rules — where `MATCH` exhaustiveness is enforced on the IR
