//! plan-176-C: `rt_resource_last_holder`'s trapped-call, trapped-element-read and
//! map-iteration tests. The shared helpers and the other tests live in the root,
//! `rt_resource_last_holder.rs`.

use super::*;

/// bug-1051: `RES v AS fs::File = sameF(a) TRAP(e) … END TRAP`, where `sameF`
/// returns its argument. The desugar's temp is bound to a closed default record of
/// its own and the call's value is stored over it. The temp is not proved to own
/// what `sameF` hands back (it does not: the record is `a`'s), so its drop frees no
/// record, and that included the default: 112 bytes on every evaluation, on the
/// success path (the store's drop of the old value) and when the handler leaves
/// without storing (the scope's drop). The same for a resource union, whose default
/// is a box and a record; with the handler taken (`CONTINUE`, and `RECOVER` of
/// another binding); and with the argument read through a record field or
/// `collections::get`, beside a control whose producer the temp does own.
#[test]
fn a_trapped_call_that_returns_its_argument_frees_the_default_record() {
    check_at_both_levels(
        "same_file",
        "FUNC sameF(h AS fs::File) AS RES fs::File\n\
         \x20 RETURN h\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES v AS fs::File = sameF(a) TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 RETURN len(fs::readAll(v)) - 5\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "same_union",
        "FUNC same(h AS Handle) AS RES Handle\n\
         \x20 RETURN h\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES v AS Handle = same(a) TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 RETURN which(v) - 5\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "handler_continues",
        "FUNC sameF(h AS fs::File, n AS Integer) AS RES fs::File\n\
         \x20 IF n > 1 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 RETURN h\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES v AS fs::File = sameF(a, j) TRAP(e)\n\
         \x20     t = t + 100\n\
         \x20     CONTINUE FOR\n\
         \x20   END TRAP\n\
         \x20   t = t + len(fs::readAll(v)) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        107,
    );
    check_at_both_levels(
        "recover_alias",
        "FUNC sameF(h AS fs::File, n AS Integer) AS RES fs::File\n\
         \x20 IF n > 1 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 RETURN h\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES b AS fs::File = openOne()\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES v AS fs::File = sameF(a, j) TRAP(e)\n\
         \x20     RECOVER b\n\
         \x20   END TRAP\n\
         \x20   t = t + len(fs::readAll(v)) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        14,
    );
    check_at_both_levels(
        "union_recover_and_continue",
        "FUNC same(h AS Handle, n AS Integer) AS RES Handle\n\
         \x20 IF n > 1 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 RETURN h\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 RES b AS Handle = openOne()\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES v AS Handle = same(a, j) TRAP(e)\n\
         \x20     RECOVER b\n\
         \x20   END TRAP\n\
         \x20   t = t + which(v) + 1\n\
         \x20 NEXT\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES w AS Handle = same(a, j) TRAP(e)\n\
         \x20     t = t + 100\n\
         \x20     CONTINUE FOR\n\
         \x20   END TRAP\n\
         \x20   t = t + which(w) + 1\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        115,
    );
    check_at_both_levels(
        "field_get_and_producer",
        "FUNC mk() AS HB\n\
         \x20 RETURN HB[openOne()]\n\
         END FUNC\n\
         FUNC sameF(h AS fs::File) AS RES fs::File\n\
         \x20 RETURN h\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET k AS HB = mk()\n\
         \x20 RES v AS fs::File = sameF(k.h) TRAP(e)\n\
         \x20   RETURN 5\n\
         \x20 END TRAP\n\
         \x20 MUT xs AS List OF RES fs::File = []\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 xs = collections::append(xs, a)\n\
         \x20 RES g AS fs::File = sameF(collections::get(xs, 0)) TRAP(e)\n\
         \x20   RETURN 6\n\
         \x20 END TRAP\n\
         \x20 RES p AS fs::File = openOne() TRAP(e)\n\
         \x20   RETURN 7\n\
         \x20 END TRAP\n\
         \x20 RETURN rd(v) + rd(g) + rd(p) + 1\n\
         END FUNC\n",
        19,
    );
}

/// bug-1047: `whichL("t", collections::get(hs, i)) TRAP(e) … END TRAP` over a `List
/// OF RES fs::File`. The inline `TRAP` lifts the fallible `get` out of the argument
/// into a temporary of its own (`bind $trap_resN = callResult collections.get(hs,
/// i)`), so the element read is no longer written straight into an argument: the
/// list counted as read back out of, lost the proof, and with it the free of its
/// own block. Either trapped read alone keeps the block (96 bytes for one element,
/// 192 for three), the failing one and the succeeding one alike. The same over a
/// resource union, a `Map`, a list floated in a loop and read in a loop with an
/// early `RETURN`, and a `RES` binding of the trapped read itself.
#[test]
fn a_trapped_element_read_does_not_keep_the_list_block() {
    check_at_both_levels(
        "two_reads",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES fs::File = [a]\n\
         \x20 MUT r AS Integer = 0\n\
         \x20 r = whichL(\"t\", collections::get(hs, 5)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 MUT q AS Integer = 0\n\
         \x20 q = whichL(\"t\", collections::get(hs, 0)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 RETURN r + q - 7\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "failing_read",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES fs::File = [a]\n\
         \x20 MUT r AS Integer = 0\n\
         \x20 r = whichL(\"t\", collections::get(hs, 5)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 RETURN r - 6\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "succeeding_read",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES fs::File = [a]\n\
         \x20 MUT q AS Integer = 0\n\
         \x20 q = whichL(\"t\", collections::get(hs, 0)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 RETURN q\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "three_elements",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES b AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES fs::File = [a, b, c]\n\
         \x20 MUT q AS Integer = 0\n\
         \x20 q = whichL(\"t\", collections::get(hs, 0)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 RETURN q\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "union_list",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS Handle = openOne()\n\
         \x20 LET hs AS List OF RES Handle = [a]\n\
         \x20 MUT r AS Integer = 0\n\
         \x20 r = whichLH(\"t\", collections::get(hs, 5)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 MUT q AS Integer = 0\n\
         \x20 q = whichLH(\"t\", collections::get(hs, 0)) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 RETURN r + q - 7\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "map",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"a\" := a }\n\
         \x20 MUT r AS Integer = 0\n\
         \x20 r = whichL(\"t\", collections::get(m, \"zz\")) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 MUT q AS Integer = 0\n\
         \x20 q = whichL(\"t\", collections::get(m, \"a\")) TRAP(e)\n\
         \x20   RECOVER 7\n\
         \x20 END TRAP\n\
         \x20 RETURN r + q - 7\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "floated_loop_return",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT hs AS List OF RES fs::File = []\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES a AS fs::File = openOne()\n\
         \x20   hs = collections::append(hs, a)\n\
         \x20 NEXT\n\
         \x20 FOR j = 0 TO 2\n\
         \x20   MUT q AS Integer = 0\n\
         \x20   q = whichL(\"t\", collections::get(hs, j)) TRAP(e)\n\
         \x20     RECOVER 7\n\
         \x20   END TRAP\n\
         \x20   t = t + q\n\
         \x20   IF j = 2 THEN\n\
         \x20     RETURN t - 8\n\
         \x20   END IF\n\
         \x20 NEXT\n\
         \x20 RETURN 0\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "bound_trapped_read",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 LET hs AS List OF RES fs::File = [a]\n\
         \x20 RES g AS fs::File = collections::get(hs, 0) TRAP(e)\n\
         \x20   RETURN 9\n\
         \x20 END TRAP\n\
         \x20 RES g2 AS fs::File = collections::get(hs, 4) TRAP(e)\n\
         \x20   RECOVER a\n\
         \x20 END TRAP\n\
         \x20 RETURN whichL(\"t\", g) + whichL(\"u\", g2) - 1\n\
         END FUNC\n",
        1,
    );
}

/// bug-1048: `FOR EACH e IN m` over a `Map OF String TO RES fs::File`, read through
/// `e.value`. As filed (the value written straight into an argument) it is balanced
/// since a field read in an argument position became a use of what it reads
/// through; the first seven cases pin that: the map literal, a map built by a
/// call, one built by `collections::set` in a loop, one iterated in a callee,
/// `e.key` alone, an inline `TRAP` in the loop body, and an early `RETURN` out of a
/// nested loop. The last two are the same shape with the value bound first (`RES f
/// = e.value`) and with a `collections::get` binding after the loop: there the map
/// kept its block (192 bytes a call) until its block was judged on its own.
#[test]
fn a_map_of_resources_iterated_through_its_values_frees_its_block() {
    check_at_both_levels(
        "literal",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   t = t + whichL(\"m\", e.value)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "built_by_call",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC mkMap() AS Map OF String TO RES fs::File\n\
         \x20 MUT m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File {}\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 m = collections::set(m, \"x\", a)\n\
         \x20 m = collections::set(m, \"y\", c)\n\
         \x20 RETURN m\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 LET m AS Map OF String TO RES fs::File = mkMap()\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   t = t + whichL(\"m\", e.value)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "built_by_set",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File {}\n\
         \x20 FOR j = 1 TO 3\n\
         \x20   RES a AS fs::File = openOne()\n\
         \x20   m = collections::set(m, \"k\" & toString(j), a)\n\
         \x20 NEXT\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   t = t + whichL(\"m\", e.value)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 2\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "iterated_in_callee",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC sumM(m AS Map OF String TO RES fs::File) AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   t = t + whichL(\"m\", e.value)\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20 RETURN sumM(m) - 1\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "key_only",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   t = t + len(e.key)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "inline_trap",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC whichT(label AS String, f AS fs::File, n AS Integer) AS Integer\n\
         \x20 IF n > 1 THEN\n\
         \x20   FAIL error(77, \"boom\")\n\
         \x20 END IF\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 MUT n AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   n = n + 1\n\
         \x20   t = t + whichT(\"m\", e.value, n) TRAP(err)\n\
         \x20     RECOVER 10\n\
         \x20   END TRAP\n\
         \x20 NEXT\n\
         \x20 RETURN t - 10\n\
         END FUNC\n",
        0,
    );
    check_at_both_levels(
        "nested_return",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR j = 1 TO 2\n\
         \x20   RES a AS fs::File = openOne()\n\
         \x20   RES c AS fs::File = openOne()\n\
         \x20   LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20   FOR EACH e IN m\n\
         \x20     t = t + whichL(\"m\", e.value)\n\
         \x20     IF t = 3 THEN\n\
         \x20       RETURN t - 2\n\
         \x20     END IF\n\
         \x20   NEXT\n\
         \x20 NEXT\n\
         \x20 RETURN t\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "value_bound",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   RES f AS fs::File = e.value\n\
         \x20   t = t + whichL(\"m\", f)\n\
         \x20 NEXT\n\
         \x20 RETURN t - 1\n\
         END FUNC\n",
        1,
    );
    check_at_both_levels(
        "get_after_loop",
        "FUNC whichL(label AS String, f AS fs::File) AS Integer\n\
         \x20 LET text AS String = fs::readAll(f)\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC whichLH(label AS String, h AS Handle) AS Integer\n\
         \x20 MATCH h\n\
         \x20   CASE fs::File(f)\n\
         \x20     LET text AS String = fs::readAll(f)\n\
         \x20     RETURN 1\n\
         \x20   CASE tcp::Socket(s)\n\
         \x20     RETURN 2\n\
         \x20 END MATCH\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC once() AS Integer\n\
         \x20 RES a AS fs::File = openOne()\n\
         \x20 RES c AS fs::File = openOne()\n\
         \x20 LET m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File { \"x\" := a, \"y\" := c }\n\
         \x20 MUT t AS Integer = 0\n\
         \x20 FOR EACH e IN m\n\
         \x20   t = t + whichL(\"m\", e.value)\n\
         \x20 NEXT\n\
         \x20 RES g AS fs::File = collections::get(m, \"y\")\n\
         \x20 RETURN t - 2 + whichL(\"g\", g)\n\
         END FUNC\n",
        1,
    );
}
