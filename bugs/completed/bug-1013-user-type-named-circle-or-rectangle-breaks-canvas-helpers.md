# bug-1013: a user `TYPE Circle` or `TYPE Rectangle` breaks the canvas helpers (internal error)

Last updated: 2026-10-10
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed (bs18-agent-j, commits 81bd28bfd RED and 1fe87584b fix)
Regression Test: the canvas unit test added in 81bd28bfd

An app-mode program that declares its own `TYPE Circle` or `TYPE Rectangle` and draws
`canvas::Circle[…]` / `canvas::Rectangle[…]` failed to build with an internal error.

## Reproduction (coordinator, 2026-10-10, main-code compiler `/tmp/main-dbg-target/release/mfb`)

```
IMPORT app
IMPORT canvas
IMPORT color

TYPE Circle
  r AS Float
END TYPE

TYPE Rectangle
  w AS Float
END TYPE

UNION Shape
  Rectangle
  Circle
END UNION

FUNC main() AS Integer
  app::setMode(app::Mode.Canvas)
  LET s AS Shape = Circle[r := 1.0]
  canvas::present([canvas::Circle[x := 1.0, y := 2.0, radius := 3.0, paint := canvas::fill(color::black)], canvas::Rectangle[x := 0.0, y := 0.0, w := 1.0, h := 1.0, paint := canvas::fill(color::black)]])
  RETURN 0
END FUNC
```

- Before: `mfb build --app` exits 1 with
  `error: canvas: the record 'Rectangle' has no field 'paint' while lowering bind pending AS Integer`.
- After (bs18-agent-j): builds, exit 0. With the user types renamed (`Circx`/`Rectx`) both compilers build.

## Cause and fix

`record_type`, `variant` and `enum_ordinal` in `func_geo_build.rs`, and `variant_tag` in
`func_carried_hashes.rs`, tried the bare spelling before `canvas.<Name>`. A bare spelling is the
user's type, so the user's record layout (or a user `UNION`'s tag) was used in place of canvas's.
The fix tries the `canvas.`-qualified spelling first and keeps the bare one as a fallback.

Found by bs18-agent-j while fixing bug-988; reproduced by the coordinator.
