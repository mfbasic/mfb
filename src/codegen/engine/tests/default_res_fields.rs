//! An inline `TRAP` binding a record that holds a `RES` field next to a function
//! value lowers on every backend.
//!
//! The function-value field makes the record neither freeable-flat nor a graph it
//! owns, so the trap temp (`$trap_valN`) gets the record's default value, built
//! field by field (`lower_default_value`). A field declared `AS RES T` carries the
//! `RES` marker in its type, which no arm of the default matched, so the build
//! failed with "native code cannot materialize default value for type 'RES
//! fs.File'" on a program the front end accepts. Both resource shapes a `RES`
//! field can name are covered: a concrete resource (the default is a closed
//! record) and a resource union (a box around one, which the record copies inline).

use crate::target::NativeBuildMode::Console;
use crate::testutil::{try_code_for_src, CodeTarget};

fn program(field: &str, open: &str) -> String {
    format!(
        "IMPORT io
IMPORT fs
IMPORT tcp

UNION Handle
  fs::File
  tcp::Socket
END UNION

TYPE Kit
  h AS RES {field}
  g AS FUNC(Integer) AS Integer
END TYPE

FUNC mkKit() AS Kit
  RES f AS fs::File = {open}
  RETURN Kit[h := f, g := LAMBDA(x AS Integer) -> x + 1]
END FUNC

FUNC main() AS Integer
  LET t AS Kit = mkKit() TRAP(e)
    RECOVER mkKit()
  END TRAP
  LET call AS FUNC(Integer) AS Integer = t.g
  io::print(toString(call(1)))
  RETURN 0
END FUNC
"
    )
}

#[test]
fn a_trapped_record_with_a_res_field_and_a_function_field_lowers() {
    for field in ["fs::File", "Handle"] {
        let source = program(field, "fs::createTempFile()");
        for target in CodeTarget::ALL {
            if let Err(error) = try_code_for_src(&source, target, Console) {
                panic!("{}: `h AS RES {field}`: {error}", target.name());
            }
        }
    }
}
