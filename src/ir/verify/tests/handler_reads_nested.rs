//! bug-797: a function-level `TRAP` handler may read only the locals
//! initialized on every failure edge reaching it — and that holds wherever in
//! the handler the read sits. The census of what a handler reads walks every
//! value of every statement kind: a `MATCH`'s scrutinee, its one-of patterns
//! and its `WHEN` guards and case bodies, both `DO` forms, `WHILE`, `FOR`'s
//! bounds, and `FOR EACH`'s iterable.
//!
//! Each `lateN` is declared after a call that can fail, so the handler can be
//! entered before it is bound, and each is read in exactly ONE place below: a
//! read the census failed to reach would be a missing diagnostic naming that
//! local. `early` is declared before anything can fail, so reading it is fine
//! everywhere.

use crate::testutil::check_src_details;

#[test]
fn a_late_local_read_anywhere_in_the_handler_is_refused() {
    let src = "\
IMPORT io
FUNC risky(s AS String) AS Integer
  RETURN toInt(s)
END FUNC
FUNC main() AS Integer
  LET early AS Integer = 1
  LET late1 AS Integer = risky(\"1\")
  LET late2 AS Integer = risky(\"2\")
  LET late3 AS Integer = risky(\"3\")
  LET late4 AS Integer = risky(\"4\")
  LET late5 AS Integer = risky(\"5\")
  LET late6 AS Integer = risky(\"6\")
  LET late7 AS Integer = risky(\"7\")
  LET late8 AS Integer = risky(\"8\")
  RETURN late1 + late2 + late3 + late4 + late5 + late6 + late7 + late8
  TRAP(e)
    MATCH late1
      CASE 1, 2
        io::print(\"low\")
      CASE 3 WHEN early > late2
        io::print(\"guarded\")
      CASE ELSE
        io::print(toString(late8))
    END MATCH
    MUT n AS Integer = early
    DO WHILE n < late3
      n = n + 1
    LOOP
    DO
      n = n - 1
    LOOP UNTIL n < late4
    WHILE n > late5
      n = n - 1
    END WHILE
    FOR i = early TO late6
      n = n + i
    NEXT
    FOR EACH x IN [late7, early]
      n = n + x
    NEXT
    RETURN early
  END TRAP
END FUNC
";
    let diagnostics = check_src_details(src);
    let mut named: Vec<String> = diagnostics
        .iter()
        .filter(|d| d.rule == "TYPE_TRAP_LOCAL_UNINITIALIZED")
        .filter_map(|d| {
            let start = d.detail.find("reads `")? + "reads `".len();
            let end = start + d.detail[start..].find('`')?;
            Some(d.detail[start..end].to_string())
        })
        .collect();
    named.sort();
    let want: Vec<String> = (1..=8).map(|n| format!("late{n}")).collect();
    assert_eq!(
        named,
        want,
        "every late local read in the handler is refused, and nothing else is: {:?}",
        diagnostics
            .iter()
            .map(|d| format!("{}: {}", d.rule, d.detail))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        diagnostics.len(),
        8,
        "no other diagnostic: {:?}",
        diagnostics.iter().map(|d| &d.rule).collect::<Vec<_>>()
    );
}
