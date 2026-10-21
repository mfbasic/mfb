//! The `-nir` dump's rarer spellings: a `--debug` module, a labeled loop exit, a
//! `LINK` function that frees its result, and the resource owners lowering never
//! produces from a verified program.
//!
//! `codegen::builtins::tests::nir_json` holds that the dump is JSON and names what
//! the program has; these are the shapes none of its programs contain.

use crate::json;
use crate::target::shared::nir::NirModule;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};

const LABELED: &str = "\
FUNC main AS Integer
  MUT n AS Integer = 0
  FOR i = 1 TO 3
    FOR j = 1 TO 3
      IF j = 2 THEN
        CONTINUE FOR i
      END IF
      IF i = 3 THEN
        EXIT FOR i
      END IF
      n = n + 1
    NEXT
  NEXT
  RETURN n
END FUNC
";

const FREEING: &str = "\
IMPORT io

RESOURCE Db CLOSE BY sql::close
RESOURCE Stmt CLOSE BY sql::finalize

LINK \"sqlite3\" AS sql
  FUNC open(path AS String) AS RES Db
    SYMBOL \"sqlite3_open\"
    ABI (path CString, db OUT CPtr) AS status CInt32
    RETURN db
    SUCCESS_ON status = 0
  END FUNC
  FUNC prepare(db AS Db, statement AS String) AS RES Stmt
    SYMBOL \"sqlite3_prepare_v2\"
    ABI (db CPtr, statement CString, nByte CInt32, stmt OUT CPtr, pzTail CPtr) AS status CInt32
    CONST nByte = -1
    CONST pzTail = NOTHING
    RETURN stmt
    SUCCESS_ON status = 0
  END FUNC
  FUNC expandedSql(stmt AS Stmt) AS String
    SYMBOL \"sqlite3_expanded_sql\"
    ABI (stmt CPtr) AS text CPtr
    RETURN text
    FREE text
      SYMBOL \"sqlite3_free\"
      ABI (ptr CPtr) AS CVoid
    END FREE
  END FUNC
  FUNC close(db AS Db) AS Nothing
    SYMBOL \"sqlite3_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
  FUNC finalize(stmt AS Stmt) AS Nothing
    SYMBOL \"sqlite3_finalize\"
    ABI (stmt CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main() AS Integer
  RES db AS Db = sql::open(\":memory:\")
  RES stmt AS Stmt = sql::prepare(db, \"SELECT 1\")
  io::print(sql::expandedSql(stmt))
  RETURN 0
END FUNC
";

fn module(source: &str) -> NirModule {
    nir_for_src(source, CodeTarget::LinuxX86_64, Console).expect("the program lowers to NIR")
}

fn dump(module: &NirModule) -> String {
    let text = module.to_json();
    json::parse_json_bounded(&text).expect("the dump is JSON");
    text
}

/// A `--debug` module says so in its dump; a normal one does not (so every
/// committed `.nir` golden is unchanged).
#[test]
fn only_a_debug_module_says_so_in_its_dump() {
    let mut debug = module(LABELED);
    assert!(!dump(&debug).contains("\"debug\""));
    debug.debug.enabled = true;
    assert!(dump(&debug).contains("\"debug\": true"));
}

/// `EXIT FOR i` / `CONTINUE FOR i` name the loop they leave -- lowering resolves the
/// label to the loop's iterator binding -- and the dump says which.
#[test]
fn a_labeled_loop_exit_and_continue_name_their_loop_in_the_dump() {
    let text = dump(&module(LABELED));
    assert!(text.contains("\"op\": \"exitLoop\", \"loop\": \"for\", \"target\": \"$for_iter0\""));
    assert!(
        text.contains("\"op\": \"continueLoop\", \"loop\": \"for\", \"target\": \"$for_iter0\"")
    );
}

/// A `LINK` function with a `FREE` block dumps the deallocator's slot and symbol.
#[test]
fn a_link_function_that_frees_its_result_dumps_the_deallocator() {
    let text = dump(&module(FREEING));
    assert!(
        text.contains("\"symbol\": \"sqlite3_free\""),
        "the FREE block's deallocator must be in the dump"
    );
}

/// A resource whose float to a returned collection is blocked dumps as such. A
/// verified program never reaches lowering with one, so the module is corrupted
/// the way `ir::verify` would have refused.
#[test]
fn a_float_blocked_resource_owner_dumps_with_its_scope() {
    let mut module = module(LABELED);
    module.functions[0].resource_owners.insert(
        "handle".to_string(),
        crate::ir::resource_escape::ResOwner::FloatBlocked("items".to_string()),
    );
    let text = dump(&module);
    assert!(text.contains("\"kind\": \"float-blocked\", \"scope\": \"items\""));
}

/// An import's parameters dump their type and whether they carry a default, and a
/// function with no recorded source file omits the `file` key rather than writing
/// an empty one.
#[test]
fn import_parameters_and_a_function_without_a_file_dump_as_they_are() {
    use crate::target::shared::nir::{NirImport, NirImportParam};
    use crate::types::ParameterType;
    let mut module = module(LABELED);
    module.imports.push(NirImport {
        package: "pkg".to_string(),
        name: "scale".to_string(),
        symbol: "_mfb_pkg_scale".to_string(),
        kind: "func".to_string(),
        isolated: false,
        params: vec![
            NirImportParam {
                type_: ParameterType::Integer,
                has_default: false,
            },
            NirImportParam {
                type_: ParameterType::String,
                has_default: true,
            },
        ],
        returns: ParameterType::Integer,
    });
    module.functions[0].file = String::new();
    let text = dump(&module);
    assert!(text.contains("{ \"type\": \"Integer\", \"hasDefault\": false }"));
    assert!(text.contains("{ \"type\": \"String\", \"hasDefault\": true }"));
    let main = text.find("\"name\": \"main\"").expect("main is dumped");
    assert!(
        !text[main..].contains("\"file\":"),
        "a function with no source file writes no `file` key"
    );
}
