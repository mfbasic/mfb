//! bug-798: attaching an inline `TRAP` turned a successful `Float` expression into
//! a failure. The desugar lifted every raising operator into its own `Checked`
//! capture, and each capture observed its `Float` result. So a transient
//! non-finite intermediate that the untrapped expression is allowed to recover
//! from (`1.0 / (1e200 * 1e200)` → `+0.0`, `mfb spec language types`) trapped,
//! and so did a comparison operand and an `a * b - c` chain that contracts to a
//! finite FMA.
//!
//! The trapped forms must agree with the untrapped contract at every `-O` level:
//!
//! * a transient intermediate recovers;
//! * a comparison operand is not observed;
//! * contraction keeps `a * 2.0 - a` finite;
//! * a real overflow, `NaN` or domain error at the boundary still reaches the
//!   handler;
//! * a `MOD` domain error inside a tree still raises before a later call's side
//!   effect;
//! * `^` overflow inside a tree recovers like any other intermediate.

#[path = "../common/mod.rs"]
mod common;

const MAIN_MFB: &str = r#"IMPORT io

FUNC identity(x AS Float) AS Float
  RETURN x
END FUNC

FUNC flag(b AS Boolean) AS Boolean
  RETURN b
END FUNC

FUNC side() AS Float
  io::print("  side effect ran")
  RETURN 1.0
END FUNC

FUNC show(label AS String, v AS Float) AS Nothing
  io::print(label & " ok " & toString(v))
END FUNC

FUNC main() AS Integer
  MUT huge AS Float = 1e200
  MUT big AS Float = 1.5e308
  MUT zero AS Float = 0.0
  MUT a AS Float = 7.0

  ' 1. recovers before the boundary
  show("recover-plain", identity(1.0 / (huge * huge)))
  LET r1 = identity(1.0 / (huge * huge)) TRAP(e)
    io::print("recover-trap TRAPPED " & toString(e.code))
    RECOVER -1.0
  END TRAP
  show("recover-trap", r1)

  ' 2. real overflow at the argument boundary
  LET r2 = identity(huge * huge) TRAP(e)
    io::print("overflow-trap trapped " & toString(e.code))
    RECOVER -1.0
  END TRAP

  ' 3. comparison is not a boundary
  LET r3 = flag(huge * huge > 1.0) TRAP(e)
    io::print("compare-trap TRAPPED " & toString(e.code))
    RECOVER FALSE
  END TRAP
  io::print("compare-trap " & toString(r3))

  ' 4. contraction a * 2.0 - a stays finite
  LET r4 = identity(big * 2.0 - big) TRAP(e)
    io::print("fma-trap TRAPPED " & toString(e.code))
    RECOVER -1.0
  END TRAP
  io::print("fma-trap ok " & toString(r4 = big))

  ' 5. MOD domain error under an operator, before a later side effect
  LET r5 = identity((a MOD zero) + side()) TRAP(e)
    io::print("mod-trap trapped " & toString(e.code))
    RECOVER -1.0
  END TRAP

  ' 6. ^ overflow under an operator recovers
  LET r6 = identity(1.0 / (huge ^ 2.0)) TRAP(e)
    io::print("pow-recover-trap TRAPPED " & toString(e.code))
    RECOVER -1.0
  END TRAP
  show("pow-recover-trap", r6)

  ' 7. ^ overflow at the boundary traps
  LET r7 = identity(huge ^ 2.0) TRAP(e)
    io::print("pow-overflow-trap trapped " & toString(e.code))
    RECOVER -1.0
  END TRAP

  ' 8. ^ domain error under an operator
  LET r8 = identity((a ^ 0.5) + 1.0) TRAP(e)
    io::print("pow-domain-trap trapped " & toString(e.code))
    RECOVER -1.0
  END TRAP

  ' 9. NaN at the boundary
  LET r9 = identity((huge * huge) - (huge * huge)) TRAP(e)
    io::print("nan-trap trapped " & toString(e.code))
    RECOVER -1.0
  END TRAP
  RETURN 0
END FUNC
"#;

#[test]
fn inline_trap_preserves_float_observation_boundaries() {
    let project = common::temp_project("bug798_float_observation", MAIN_MFB);
    let expected = vec![
        "recover-plain ok 0.00",
        "recover-trap ok 0.00",
        "overflow-trap trapped 77050015",
        "compare-trap TRUE",
        "fma-trap ok TRUE",
        "mod-trap trapped 77050012",
        "pow-recover-trap ok 0.00",
        "pow-overflow-trap trapped 77050015",
        "pow-domain-trap trapped 77050012",
        "nan-trap trapped 77050015",
    ];
    for level in ["0", "1", "2", "3"] {
        let executable = common::build_project_opt(&project, level);
        let output = common::run_with_stdin(&executable, b"");
        assert_eq!(output.lines().collect::<Vec<_>>(), expected, "-O{level}");
    }
}
