//! bug-874: `typeName(Color.Green)` — an enum member written as the argument — is the
//! enum's name, in every position, exactly like `typeName(c)` for `c AS Color`. It
//! used to abort the build with "native code cannot determine typeName argument type".

#[path = "../common/mod.rs"]
mod common;

use common::{build_project, run_with_stdin, temp_project};

const SOURCE: &str = "IMPORT io

ENUM Color
  Red, Green
END ENUM

ENUM Size
  Small, Large
END ENUM

FUNC name(s AS String) AS String
  RETURN s
END FUNC

FUNC main AS Integer
  io::print(typeName(Color.Green))
  LET s = typeName(Size.Large)
  io::print(s)
  io::print(typeName((Color.Red)))
  io::print(name(typeName(Size.Small)))
  LET c AS Color = Color.Red
  io::print(typeName(c))
  RETURN 0
END FUNC
";

#[test]
fn typename_of_an_enum_member_literal_is_the_enum_name() {
    let exe = build_project(&temp_project("typename_enum_member", SOURCE));
    let out = run_with_stdin(&exe, b"");
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        ["Color", "Size", "Color", "Size", "Color"]
    );
}
