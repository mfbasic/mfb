//! plan-176-C: `rt_scope_drop_leaks`' address (bug-599), recursive-loop and
//! condition-temp (bug-620 / bug-621) tests. `condition_temp_report`,
//! `assert_condition_temps_freed` and the other shared helpers live in the root,
//! `rt_scope_drop_leaks.rs`.

use super::*;

// ---------------------------------------------------------------- bug-599

/// bug-599 positive pin. The `net::Address` builder now frees its `inet_ntop`
/// scratch buffer, and `net::lookup` frees the temporary record whose two words it
/// copies into the list, so the failure direction is a block freed while an
/// `Address` still points at it: a host that reads back empty or garbled, or a
/// port read from a clobbered record. The builder parks the record pointer across
/// the free and writes the PORT after it, so the port is the sharpest witness.
///
/// Every value is read back after churn that would recycle a freed block:
///
/// * `kept` — an element copied out of a list that was the callee's local, read
///   after 500 further lookups;
/// * `total` — each element read both by iteration and by index inside the loop;
/// * `iter` — the plain `FOR EACH` output;
/// * `tcp`/`datagram` — the builder's other callers (`localAddress`,
///   `udp::receive`'s nested `from`);
/// * `trapped`/`propagated` — bug-593's error shapes on a failing lookup, which
///   must still report their code and origin line.
///
/// The error MESSAGE is not asserted: resolver wording is the platform's. Measured
/// identical, line for line, against the compiler before the change.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const SHAPE_599_ADDRESS_VALUES: &str = "IMPORT io\n\
IMPORT net\n\
IMPORT tcp\n\
IMPORT udp\n\
IMPORT collections\n\
FUNC firstOf(host AS String, port AS Integer) AS net::Address\n\
  LET found = net::lookup(host, port)\n\
  RETURN collections::get(found, 0)\n\
END FUNC\n\
FUNC failing(host AS String) AS Integer\n\
  LET xs = net::lookup(host)\n\
  RETURN len(xs)\n\
END FUNC\n\
FUNC main AS Integer\n\
  LET kept = firstOf(\"127.0.0.1\", 8080)\n\
  MUT total AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < 500\n\
    LET xs = net::lookup(\"127.0.0.1\", 443)\n\
    FOR EACH a IN xs\n\
      total = total + len(a.host) + a.port\n\
    NEXT\n\
    LET e0 = collections::get(xs, 0)\n\
    total = total + e0.port\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"kept \" & kept.host & \":\" & toString(kept.port))\n\
  io::print(\"total \" & toString(total))\n\
  FOR EACH a IN net::lookup(\"127.0.0.1\", 53)\n\
    io::print(\"iter \" & a.host & \":\" & toString(a.port))\n\
  NEXT\n\
  RES server = tcp::listen(\"127.0.0.1\", 0)\n\
  MUT stable AS Boolean = TRUE\n\
  LET first = tcp::localAddress(server)\n\
  MUT j AS Integer = 0\n\
  WHILE j < 500\n\
    LET b = tcp::localAddress(server)\n\
    IF b.port <> first.port OR b.host <> \"127.0.0.1\" THEN\n\
      stable = FALSE\n\
    END IF\n\
    j = j + 1\n\
  END WHILE\n\
  io::print(\"tcp \" & first.host & \" stable=\" & toString(stable) & \" portOk=\" & toString(first.port > 0))\n\
  RES sock = udp::bind(\"127.0.0.1\", 0)\n\
  LET at = udp::localAddress(sock)\n\
  RES peer = udp::bind(\"127.0.0.1\", 0)\n\
  udp::send(peer, at, \"ping\")\n\
  LET dg = udp::receive(sock, 5000)\n\
  io::print(\"datagram \" & dg.from.host & \" \" & toString(len(dg.bytes)) & \" portOk=\" & toString(dg.from.port > 0))\n\
  LET bad = net::lookup(\"no-such-host.invalid\") TRAP(e)\n\
    io::print(\"trapped code=\" & toString(e.code))\n\
    RECOVER []\n\
  END TRAP\n\
  io::print(\"bad \" & toString(len(bad)))\n\
  LET n = failing(\"no-such-host.invalid\") TRAP(e2)\n\
    io::print(\"propagated code=\" & toString(e2.code) & \" line=\" & toString(e2.source.line))\n\
    RECOVER -1\n\
  END TRAP\n\
  io::print(\"n \" & toString(n))\n\
  RETURN 0\n\
END FUNC\n";

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn every_address_reads_back_after_its_builder_scratch_is_freed() {
    let expected = [
        "kept 127.0.0.1:8080".to_string(),
        // 500 iterations x (len("127.0.0.1") + 443 iterated + 443 indexed).
        format!("total {}", 500 * (9 + 443 + 443)),
        "iter 127.0.0.1:53".to_string(),
        "tcp 127.0.0.1 stable=TRUE portOk=TRUE".to_string(),
        "datagram 127.0.0.1 4 portOk=TRUE".to_string(),
        "trapped code=77070002".to_string(),
        "bad 0".to_string(),
        "propagated code=77070002 line=11".to_string(),
        "n -1".to_string(),
    ]
    .join("\n");
    let project = common::temp_project("b599_address_values", SHAPE_599_ADDRESS_VALUES);
    let exe = common::build_project(&project);
    for run in 1..=5 {
        let output = std::process::Command::new(&exe)
            .current_dir(&project)
            .output()
            .expect("run the net::Address read-back probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected,
            "run {run}: a net::Address read back wrong. bug-599 frees the address \
             builder's scratch buffer and lookup's temporary record — a wrong \
             `host` means a freed block was still referenced, a wrong `port` that \
             the record pointer was not restored across the free"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-599 / plan-132

/// plan-132 (bug-599): once `net::Address` and `udp::Datagram` use the ordinary flat
/// record layout, a `List OF net::Address`, a lone `net::Address`, a `Datagram` and
/// the host `String` inside each have an owner and are freed. Peak RSS on the
/// compiler before plan-132 (macOS, 200k -> 400k iterations): `net::lookup`
/// 99.8 -> 198.3 MB, `tcp::localAddress` 38.0 -> 74.9 MB, a user-built list of two
/// copies 196.4 -> 391.8 MB.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const SHAPE_599_LOOKUP_LOOP: &str = "IMPORT io\nIMPORT net\n\
FUNC main AS Integer\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET xs = net::lookup(\"127.0.0.1\")\n\
    n = n + len(xs)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n \" & toString(n))\n\
  RETURN 0\n\
END FUNC\n";

#[cfg(any(target_os = "macos", target_os = "linux"))]
const SHAPE_599_LOCAL_ADDRESS_LOOP: &str = "IMPORT io\nIMPORT net\nIMPORT tcp\n\
FUNC main AS Integer\n\
  RES server = tcp::listen(\"127.0.0.1\", 0)\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET b = tcp::localAddress(server)\n\
    n = n + len(b.host)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n \" & toString(n))\n\
  RETURN 0\n\
END FUNC\n";

#[cfg(any(target_os = "macos", target_os = "linux"))]
const SHAPE_599_USER_BUILT_LIST_LOOP: &str = "IMPORT io\nIMPORT net\nIMPORT collections\n\
FUNC main AS Integer\n\
  LET a = collections::get(net::lookup(\"127.0.0.1\"), 0)\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT ys AS List OF net::Address = []\n\
    ys = collections::append(ys, a)\n\
    ys = collections::append(ys, a)\n\
    n = n + len(ys)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n \" & toString(n))\n\
  RETURN 0\n\
END FUNC\n";

#[cfg(any(target_os = "macos", target_os = "linux"))]
const SHAPE_599_DATAGRAM_LOOP: &str = "IMPORT io\nIMPORT net\nIMPORT udp\n\
FUNC main AS Integer\n\
  RES sock = udp::bind(\"127.0.0.1\", 0)\n\
  LET at = udp::localAddress(sock)\n\
  RES peer = udp::bind(\"127.0.0.1\", 0)\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    udp::send(peer, at, \"ping\")\n\
    LET dg = udp::receive(sock, 64)\n\
    n = n + len(dg.bytes) + len(dg.from.host)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n \" & toString(n))\n\
  RETURN 0\n\
END FUNC\n";

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_net_lookup_runs_at_constant_rss() {
    assert_flat("b599_lookup_loop", SHAPE_599_LOOKUP_LOOP, 200_000, 400_000);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_local_address_runs_at_constant_rss() {
    assert_flat(
        "b599_local_address_loop",
        SHAPE_599_LOCAL_ADDRESS_LOOP,
        200_000,
        400_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_user_built_address_list_runs_at_constant_rss() {
    assert_flat(
        "b599_user_built_list_loop",
        SHAPE_599_USER_BUILT_LIST_LOOP,
        200_000,
        400_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_udp_receive_runs_at_constant_rss() {
    assert_flat(
        "b599_datagram_loop",
        SHAPE_599_DATAGRAM_LOOP,
        200_000,
        400_000,
    );
}

// ---------------------------------------------------------------- shape C / plan-134-G

/// bug-536 shape C, union form: a recursive-union value bound each iteration. Before
/// plan-134-G nothing freed a value whose type reaches a type cycle — 52.7 MB at 400k
/// iterations, 104.3 MB at 800k (plan-134-A §2.2, `c_union_rss`).
const SHAPE_C_UNION_BIND: &str = r#"IMPORT io
IMPORT json
FUNC main() AS Nothing
  MUT i AS Integer = 0
  WHILE i < {N}
    LET v AS json::Json = json::JsonNull[NOTHING]
    i = i + 1
  END WHILE
  io::print("done")
END FUNC
"#;

/// bug-536 shape C, record form (`c_record_rss`: 105.1 MB at 400k, 209.1 MB at 800k).
const SHAPE_C_RECORD_BIND: &str = r#"IMPORT io
TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE
FUNC main() AS Nothing
  MUT i AS Integer = 0
  MUT acc AS Integer = 0
  WHILE i < {N}
    LET nd AS Node = Node[kids := [], tag := i]
    acc = acc + nd.tag
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END FUNC
"#;

/// A recursive local overwritten each iteration: the old graph is the `Assign` free's.
const SHAPE_C_REASSIGN: &str = r#"IMPORT io
TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE
FUNC main() AS Nothing
  MUT cur AS Node = Node[kids := [], tag := 0]
  MUT i AS Integer = 0
  WHILE i < {N}
    cur = Node[kids := [], tag := i]
    i = i + 1
  END WHILE
  io::print("tag=" & toString(cur.tag))
END FUNC
"#;

/// A recursive global overwritten each iteration: the `StoreGlobal` old-value free.
const SHAPE_C_GLOBAL: &str = r#"IMPORT io
TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE
MUT GN AS Node = Node[kids := [], tag := 0]
FUNC main() AS Nothing
  MUT i AS Integer = 0
  WHILE i < {N}
    GN = Node[kids := [], tag := i]
    i = i + 1
  END WHILE
  io::print("tag=" & toString(GN.tag))
END FUNC
"#;

/// An unbound recursive temporary: a function's fresh result consumed as another call's
/// argument inside one expression, freed at the end of the statement. (The
/// `len(json::stringify(json::parse(t)))` form of this case belongs to plan-134-H: a
/// `--debug` build leaks the same 2 912 B per iteration whether `json::parse`'s result is
/// bound or unbound — blocks left inside json's list-building helpers, not the temp.)
const SHAPE_C_UNBOUND_TEMP: &str = r#"IMPORT io
TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE
FUNC mk(t AS Integer) AS Node
  RETURN Node[kids := [Node[kids := [], tag := t]], tag := t]
END FUNC
FUNC total(n AS Node) AS Integer
  MUT sum AS Integer = n.tag
  FOR EACH k IN n.kids
    sum = sum + total(k)
  NEXT
  RETURN sum
END FUNC
FUNC main() AS Nothing
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    acc = acc + total(mk(i))
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END FUNC
"#;

/// A recursive value captured by a non-escaping closure: the closure drop frees the
/// captured graph, and the capture's source is either copied or moved out of.
const SHAPE_C_CLOSURE: &str = r#"IMPORT io
TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE
FUNC main() AS Nothing
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET cap AS Node = Node[kids := [], tag := i]
    LET f AS FUNC(Integer) AS Integer = LAMBDA(k AS Integer) -> k + cap.tag
    acc = acc + f(1)
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END FUNC
"#;

/// bug-538's owned copy out of a container: `collections::get` of a recursive element.
const SHAPE_C_GET: &str = r#"IMPORT io
IMPORT collections
TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE
FUNC main() AS Nothing
  LET xs AS List OF Node = [Node[kids := [], tag := 7]]
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET e AS Node = collections::get(xs, 0)
    acc = acc + e.tag
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END FUNC
"#;

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_recursive_union_bind_runs_at_constant_rss() {
    assert_flat(
        "c_recursive_union_bind",
        SHAPE_C_UNION_BIND,
        400_000,
        800_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_recursive_record_bind_runs_at_constant_rss() {
    assert_flat(
        "c_recursive_record_bind",
        SHAPE_C_RECORD_BIND,
        400_000,
        800_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_recursive_reassignment_runs_at_constant_rss() {
    assert_flat("c_recursive_reassign", SHAPE_C_REASSIGN, 400_000, 800_000);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_recursive_global_overwrite_runs_at_constant_rss() {
    assert_flat("c_recursive_global", SHAPE_C_GLOBAL, 400_000, 800_000);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_unbound_recursive_temp_runs_at_constant_rss() {
    assert_flat(
        "c_recursive_unbound_temp",
        SHAPE_C_UNBOUND_TEMP,
        400_000,
        800_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_recursive_closure_capture_runs_at_constant_rss() {
    assert_flat("c_recursive_closure", SHAPE_C_CLOSURE, 400_000, 800_000);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_recursive_get_result_runs_at_constant_rss() {
    assert_flat("c_recursive_get", SHAPE_C_GET, 400_000, 800_000);
}

// ---------------------------------------------------------------- shape C / plan-134-H

/// plan-134-H: parsing the same 480 003-byte JSON document K times costs its tree once.
/// `tools/recursive-value-bench/programs/json_repeat` with K = {N}: 204 / 402 / 799 MB at
/// K = 1 / 2 / 4 before plan-134 (plan-134-A §2.1).
const SHAPE_C_JSON_REPEAT: &str = r#"IMPORT io
IMPORT json
FUNC main() AS Nothing
  MUT text AS String = "["
  MUT j AS Integer = 0
  WHILE j < 20000
    text = text & "{\u{22}a\u{22}:[1,2,3],\u{22}b\u{22}:\u{22}xyz\u{22}},"
    j = j + 1
  END WHILE
  text = text & "0]"
  MUT i AS Integer = 0
  WHILE i < {N}
    LET v AS json::Json = json::parse(text)
    i = i + 1
  END WHILE
  io::print("bytes=" & toString(len(text)))
END FUNC
"#;

/// plan-134-H: a 100 000-character `regex::findAll` repeated K times costs its matcher graphs
/// once. `tools/recursive-value-bench/programs/regex_repeat` with K = {N}: 345 / 673 / 1329 MB
/// at K = 1 / 2 / 4 before plan-134.
const SHAPE_C_REGEX_REPEAT: &str = r#"IMPORT io
IMPORT regex
FUNC main() AS Nothing
  MUT subject AS String = ""
  MUT j AS Integer = 0
  WHILE j < 10000
    subject = subject & "abcab1234 "
    j = j + 1
  END WHILE
  MUT hits AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    hits = len(regex::findAll(subject, "[a-c]+[0-9]+"))
    i = i + 1
  END WHILE
  io::print("hits=" & toString(hits))
END FUNC
"#;

/// plan-134-H (from plan-134-G): the json form of the unbound recursive temp. After G a
/// `--debug` build leaves `live_bytes 2912000` per 1 000 iterations whether `json::parse`'s
/// result is bound or not — blocks left inside json's list-building helpers.
const SHAPE_C_JSON_UNBOUND_TEMP: &str = r#"IMPORT io
IMPORT json
FUNC main() AS Nothing
  LET t AS String = "[1,{\u{22}a\u{22}:[2,3]}]"
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    acc = acc + len(json::stringify(json::parse(t)))
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END FUNC
"#;

/// plan-134 (final gate): an inline-`TRAP`ped `json::parse` of a document with nested
/// children, both the Ok and the RECOVER path. The `Result` wrapper holds the value's top
/// block inline and the graph below it: after plan-134-G the callee's block was
/// graph-dropped under the wrapper (a use-after-free, `rt_union_trap_bind_default`), and
/// the Ok wrapper — never released for a block payload (bug-593 `ErrorOnly`) — kept its
/// graph for the life of the process.
const SHAPE_C_JSON_TRAP_RECOVER: &str = r#"IMPORT io
IMPORT json
FUNC main() AS Nothing
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET d AS json::Json = json::parse("[1,[2,3],{\u{22}a\u{22}:\u{22}xyz\u{22}}]") TRAP(e)
      RECOVER json::JsonNull[NOTHING]
    END TRAP
    LET bad AS json::Json = json::parse("[oops") TRAP(e)
      RECOVER json::JsonNull[NOTHING]
    END TRAP
    acc = acc + len(json::stringify(d)) + len(json::stringify(bad))
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END FUNC
"#;

/// plan-134 (final gate): an inline-`TRAP` bind of a flat data union. The desugar's slot
/// starts as the union's synthesized default, whose variant record was byte-copied into
/// the union and never freed — 16 B per bind, the same on the pre-plan compiler.
const SHAPE_TRAP_UNION_DEFAULT: &str = r#"IMPORT io
TYPE Circle
  radius AS Integer
END TYPE
TYPE Rect
  w AS Integer
  h AS Integer
END TYPE
UNION Shape
  Circle
  Rect
END UNION
FUNC makeShape(n AS Integer) AS Shape
  IF n < 0 THEN FAIL error(90004440, "negative")
  RETURN Circle[n]
END FUNC
FUNC score(s AS Shape) AS Integer
  MATCH s
    CASE Circle(c)
      RETURN c.radius
    CASE Rect(r)
      RETURN r.w + r.h
  END MATCH
END FUNC
FUNC main() AS Nothing
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET ok AS Shape = makeShape(7) TRAP(e)
      RECOVER Rect[1, 2]
    END TRAP
    LET bad AS Shape = makeShape(-1) TRAP(e)
      RECOVER Rect[20, 22]
    END TRAP
    acc = acc + score(ok) + score(bad)
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END FUNC
"#;

/// plan-134 (final gate): a fresh temporary passed to a call that FAILs. The statement-end
/// free is jumped past by the call's error exit — 48 B per failure for the list literal,
/// 32 B for the concat, the same on the pre-plan compiler.
const SHAPE_FAILING_CALL_TEMP_ARGUMENT: &str = r#"IMPORT io
FUNC boom(n AS Integer, xs AS List OF Integer) AS Integer
  IF n >= 0 THEN FAIL error(90004440, "boom")
  RETURN n + len(xs)
END FUNC
FUNC boomS(n AS Integer, s AS String) AS Integer
  IF n >= 0 THEN FAIL error(90004440, "boom")
  RETURN n + len(s)
END FUNC
FUNC holder(n AS Integer) AS Integer
  LET v AS Integer = boom(n, [])
  RETURN v + boomS(n, "a" & toString(n))
END FUNC
FUNC tryHolder(n AS Integer) AS Integer
  RETURN holder(n)
  TRAP(e)
    RETURN 1
  END TRAP
END FUNC
FUNC tryHolderS(n AS Integer) AS Integer
  RETURN boomS(n, "a" & toString(n))
  TRAP(e)
    RETURN len(e.message)
  END TRAP
END FUNC
FUNC main() AS Nothing
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    acc = acc + tryHolder(i) + tryHolderS(i)
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END FUNC
"#;

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_inline_trap_union_bind_runs_at_constant_rss() {
    assert_flat(
        "trap_union_default",
        SHAPE_TRAP_UNION_DEFAULT,
        400_000,
        800_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_failing_call_with_a_temp_argument_runs_at_constant_rss() {
    assert_flat(
        "failing_call_temp_argument",
        SHAPE_FAILING_CALL_TEMP_ARGUMENT,
        400_000,
        800_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_trapped_recursive_json_parse_runs_at_constant_rss() {
    assert_flat(
        "c_recursive_json_trap_recover",
        SHAPE_C_JSON_TRAP_RECOVER,
        200_000,
        400_000,
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_repeated_json_parse_of_one_document_runs_at_constant_rss() {
    assert_flat("c_recursive_json_repeat", SHAPE_C_JSON_REPEAT, 1, 4);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_repeated_regex_find_all_runs_at_constant_rss() {
    assert_flat("c_recursive_regex_repeat", SHAPE_C_REGEX_REPEAT, 1, 4);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_looped_unbound_recursive_json_temp_runs_at_constant_rss() {
    assert_flat(
        "c_recursive_json_unbound_temp",
        SHAPE_C_JSON_UNBOUND_TEMP,
        400_000,
        800_000,
    );
}

// ------------------------------------------------- bug-620 / bug-621: condition temps

/// bug-620's reproduction: the one-line `IF … THEN RETURN` leaves past the `IF`'s drop.
#[cfg(unix)]
#[test]
fn an_if_condition_temp_is_freed_when_the_branch_returns() {
    assert_condition_temps_freed(
        "cond_if_return",
        "FUNC g(s AS String) AS Integer\n  IF strings::lower(s) <> \"zz\" THEN RETURN 0\n  RETURN 1\nEND FUNC",
        "g(\"p\")",
        0,
    );
}

/// bug-620 `r7_block`: the block form.
#[cfg(unix)]
#[test]
fn a_block_if_condition_temp_is_freed_when_the_branch_returns() {
    assert_condition_temps_freed(
        "cond_if_block_return",
        "FUNC g(s AS String) AS Integer\n  IF strings::lower(s) <> \"zz\" THEN\n    RETURN 1\n  END IF\n  RETURN 0\nEND FUNC",
        "g(\"p\")",
        1,
    );
}

/// bug-620: an `ELSEIF` is a nested `If` in the else body — same hole.
#[cfg(unix)]
#[test]
fn an_elseif_condition_temp_is_freed_when_the_branch_returns() {
    assert_condition_temps_freed(
        "cond_elseif_return",
        "FUNC g(s AS String) AS Integer\n  IF len(s) > 5 THEN\n    RETURN 2\n  ELSEIF strings::lower(s) <> \"zz\" THEN\n    RETURN 1\n  END IF\n  RETURN 0\nEND FUNC",
        "g(\"p\")",
        1,
    );
}

/// bug-620: the returned value is itself a fresh block, so the enclosing condition temp is
/// freed alongside an escaping value.
#[cfg(unix)]
#[test]
fn an_if_condition_temp_is_freed_when_the_branch_returns_a_fresh_string() {
    assert_condition_temps_freed(
        "cond_if_return_fresh",
        "FUNC h(s AS String) AS String\n  IF strings::lower(s) <> \"zz\" THEN RETURN strings::upper(s) & \"!\"\n  RETURN s\nEND FUNC",
        "len(h(\"p\"))",
        2,
    );
}

/// bug-620 `ct_close`: a record copy in the condition, left by `EXIT DO`.
#[cfg(unix)]
#[test]
fn an_if_condition_record_temp_is_freed_on_exit_do() {
    assert_condition_temps_freed(
        "cond_if_exit_do",
        "FUNC g(name AS String) AS Integer\n  LET stack AS List OF Tag = [Tag[\"a\", 1], Tag[\"b\", 2], Tag[\"c\", 3]]\n  MUT k AS Integer = -1\n  MUT s AS Integer = 2\n  DO WHILE s >= 0\n    IF collections::get(stack, s).tag = name THEN\n      k = s\n      EXIT DO\n    END IF\n    s = s - 1\n  LOOP\n  RETURN k\nEND FUNC",
        "g(\"b\")",
        1,
    );
}

/// bug-620: `EXIT FOR`.
#[cfg(unix)]
#[test]
fn an_if_condition_temp_is_freed_on_exit_for() {
    assert_condition_temps_freed(
        "cond_if_exit_for",
        "FUNC g(s AS String) AS Integer\n  MUT k AS Integer = 0\n  FOR j = 1 TO 3\n    IF strings::lower(s) = \"p\" THEN EXIT FOR\n    k = k + j\n  NEXT\n  RETURN k\nEND FUNC",
        "g(\"P\")",
        0,
    );
}

/// bug-620: `EXIT WHILE`.
#[cfg(unix)]
#[test]
fn an_if_condition_temp_is_freed_on_exit_while() {
    assert_condition_temps_freed(
        "cond_if_exit_while",
        "FUNC g(s AS String) AS Integer\n  MUT k AS Integer = 0\n  WHILE k < 3\n    IF strings::lower(s) = \"p\" THEN EXIT WHILE\n    k = k + 1\n  END WHILE\n  RETURN k\nEND FUNC",
        "g(\"P\")",
        0,
    );
}

/// bug-620: `CONTINUE WHILE`, taken on every pass.
#[cfg(unix)]
#[test]
fn an_if_condition_temp_is_freed_on_continue_while() {
    assert_condition_temps_freed(
        "cond_if_continue_while",
        "FUNC g(s AS String) AS Integer\n  MUT k AS Integer = 0\n  MUT n AS Integer = 0\n  WHILE k < 3\n    k = k + 1\n    IF strings::lower(s) = \"p\" THEN CONTINUE WHILE\n    n = n + 1\n  END WHILE\n  RETURN k + n\nEND FUNC",
        "g(\"P\")",
        3,
    );
}

/// bug-620: `CONTINUE FOR`.
#[cfg(unix)]
#[test]
fn an_if_condition_temp_is_freed_on_continue_for() {
    assert_condition_temps_freed(
        "cond_if_continue_for",
        "FUNC g(s AS String) AS Integer\n  MUT n AS Integer = 0\n  FOR j = 1 TO 3\n    IF strings::lower(s) = \"p\" THEN CONTINUE FOR\n    n = n + 1\n  NEXT\n  RETURN n\nEND FUNC",
        "g(\"P\")",
        0,
    );
}

/// bug-620: a `CONTINUE WHILE` from inside a `FOR` leaves two `IF`s and the `FOR`, and must
/// free both conditions' temps.
#[cfg(unix)]
#[test]
fn a_continue_out_of_a_nested_loop_frees_every_enclosing_condition_temp() {
    assert_condition_temps_freed(
        "cond_nested_continue",
        "FUNC g(s AS String) AS Integer\n  MUT k AS Integer = 0\n  WHILE k < 3\n    k = k + 1\n    IF strings::lower(s) = \"p\" THEN\n      FOR j = 1 TO 2\n        IF strings::upper(s) = \"P\" THEN CONTINUE WHILE\n      NEXT\n    END IF\n  END WHILE\n  RETURN k\nEND FUNC",
        "g(\"P\")",
        3,
    );
}

/// bug-621's reproduction: `DO WHILE`, nine passes per call.
#[cfg(unix)]
#[test]
fn a_do_while_condition_temp_is_freed_on_every_pass() {
    assert_condition_temps_freed(
        "cond_do_while",
        "FUNC g(s AS String) AS Integer\n  LET n AS Integer = len(s)\n  MUT i AS Integer = 0\n  DO WHILE i < n AND strings::mid(s, i, 1) <> \"=\"\n    i = i + 1\n  LOOP\n  RETURN i\nEND FUNC",
        "g(\"abcdefgh=\")",
        8,
    );
}

/// bug-621 `r10_whileend`: `WHILE … END WHILE`.
#[cfg(unix)]
#[test]
fn a_while_condition_temp_is_freed_on_every_pass() {
    assert_condition_temps_freed(
        "cond_while",
        "FUNC g(s AS String) AS Integer\n  LET n AS Integer = len(s)\n  MUT i AS Integer = 0\n  WHILE i < n AND strings::mid(s, i, 1) <> \"=\"\n    i = i + 1\n  END WHILE\n  RETURN i\nEND FUNC",
        "g(\"abcdefgh=\")",
        8,
    );
}

/// bug-621: `DO … LOOP UNTIL` tests its condition after the body.
#[cfg(unix)]
#[test]
fn a_loop_until_condition_temp_is_freed_on_every_pass() {
    assert_condition_temps_freed(
        "cond_loop_until",
        "FUNC g(s AS String) AS Integer\n  MUT i AS Integer = 0\n  DO\n    i = i + 1\n  LOOP UNTIL strings::mid(s, i, 1) = \"=\"\n  RETURN i\nEND FUNC",
        "g(\"abcdefgh=\")",
        8,
    );
}

/// bug-621: a record copy in the condition.
#[cfg(unix)]
#[test]
fn a_while_condition_record_temp_is_freed_on_every_pass() {
    assert_condition_temps_freed(
        "cond_while_record",
        "FUNC g(name AS String) AS Integer\n  LET xs AS List OF Tag = [Tag[\"a\", 1], Tag[\"b\", 2], Tag[\"c\", 3], Tag[\"d\", 4]]\n  MUT i AS Integer = 0\n  WHILE i < 4 AND collections::get(xs, i).tag <> name\n    i = i + 1\n  END WHILE\n  RETURN i\nEND FUNC",
        "g(\"d\")",
        3,
    );
}

/// POSITIVE pins that were flat before bug-620/621: a false condition falls through to the
/// `IF`'s own drop; a `LET`-bound temp is an owned local; a `MATCH` scrutinee is bound to a
/// `$match` local; a `FOR … TO` bound is evaluated once into a synthetic local.
#[cfg(unix)]
#[test]
fn condition_temps_that_already_had_an_owner_stay_flat() {
    assert_condition_temps_freed(
        "cond_if_false",
        "FUNC g(s AS String) AS Integer\n  IF strings::lower(s) <> \"zz\" THEN RETURN 0\n  RETURN 1\nEND FUNC",
        "g(\"ZZ\")",
        1,
    );
    assert_condition_temps_freed(
        "cond_bound_first",
        "FUNC g(s AS String) AS Integer\n  LET l AS String = strings::lower(s)\n  IF l <> \"zz\" THEN RETURN 0\n  RETURN 1\nEND FUNC",
        "g(\"p\")",
        0,
    );
    assert_condition_temps_freed(
        "cond_match_scrutinee",
        "FUNC g(s AS String) AS Integer\n  MATCH strings::lower(s)\n    CASE \"p\"\n      RETURN 1\n    CASE ELSE\n      RETURN 0\n  END MATCH\n  RETURN 2\nEND FUNC",
        "g(\"P\")",
        1,
    );
    assert_condition_temps_freed(
        "cond_for_bound",
        "FUNC g(s AS String) AS Integer\n  MUT n AS Integer = 0\n  FOR j = 1 TO len(strings::lower(s))\n    n = n + j\n  NEXT\n  RETURN n\nEND FUNC",
        "g(\"abcdefgh\")",
        36,
    );
}

/// bug-620, found while fixing it: a `FOR EACH` over a fresh collection keeps it as a pending
/// temp of the loop statement, so a `RETURN` from the body — which never reaches that
/// statement's end drop — leaked the whole list (192 B per call for a three-item split). The
/// returned item is a copy, so the list is freed under the escaping-value guard.
#[cfg(unix)]
#[test]
fn a_return_from_a_for_each_frees_its_fresh_collection() {
    assert_condition_temps_freed(
        "for_each_return_item",
        "FUNC pick(s AS String) AS String\n  FOR EACH x IN strings::split(s, \",\")\n    IF strings::lower(x) = \"b\" THEN RETURN x\n  NEXT\n  RETURN \"\"\nEND FUNC",
        "len(pick(\"a,B,c\"))",
        1,
    );
    assert_condition_temps_freed(
        "for_each_return_derived",
        "FUNC g(s AS String) AS Integer\n  FOR EACH x IN strings::split(s, \",\")\n    IF x = \"b\" THEN RETURN len(x & \"!\")\n  NEXT\n  RETURN 0\nEND FUNC",
        "g(\"a,b,c\")",
        2,
    );
}

/// bug-620 × bug-621: a `RETURN` from a `WHILE` body whose condition allocates on every pass
/// returns a fresh value built from the same input.
#[cfg(unix)]
#[test]
fn a_return_from_a_while_with_a_condition_temp_frees_every_block() {
    assert_condition_temps_freed(
        "while_condition_return",
        "FUNC g(s AS String) AS String\n  MUT i AS Integer = 0\n  WHILE strings::mid(s, i, 1) <> \"=\"\n    IF strings::mid(s, i, 1) = \"c\" THEN RETURN strings::left(s, i)\n    i = i + 1\n  END WHILE\n  RETURN s\nEND FUNC",
        "len(g(\"abcd=\"))",
        2,
    );
}

// ------------------------------------------------- bug-695: unbound resource temporaries

/// bug-695: a resource produced straight into a container — never bound `RES` — is
/// owned like a named binding in that position (spec 15.6) and closed once. Before the
/// fix none of these was ever closed: each iteration leaked the record (and the open
/// descriptor, or the running worker). Growth 0 between 50 and 100 iterations means
/// every one of them was dropped, since the record is freed only after its close.
#[cfg(unix)]
#[test]
fn a_resource_temporary_in_a_container_is_closed() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let file_shapes = [
        (
            "b695_list_append",
            "    MUT pool AS List OF RES fs::File = []\n    pool = collections::append(pool, fs::openFile(\"{P}\"))\n    total = total + len(pool)\n",
        ),
        (
            "b695_list_literal",
            "    LET pool AS List OF RES fs::File = [fs::openFile(\"{P}\"), fs::openFile(\"{P}\")]\n    total = total + len(pool) - 1\n",
        ),
        (
            "b695_record_literal",
            "    LET h AS H = H[f := fs::openFile(\"{P}\")]\n    total = total + 1\n",
        ),
    ];
    for (name, body) in file_shapes {
        let body = body.replace("{P}", path);
        let source = format!(
            "IMPORT collections\nIMPORT fs\nIMPORT io\nTYPE H\n  f AS RES fs::File\nEND TYPE\n\
FUNC main() AS Nothing\n  MUT total AS Integer = 0\n  MUT i AS Integer = 0\n  WHILE i < {{N}}\n{body}    i = i + 1\n  END WHILE\n  io::print(\"total=\" & toString(total))\nEND FUNC\n"
        );
        let grew = thread_loop_growth(name, &source, "total=50", "total=100");
        assert_eq!(grew, 0, "{name}: main-arena live_bytes grew {grew} B");
    }
    let thread_shapes = [
        (
            "b695_thread_pool",
            "    MUT pool AS List OF RES Thread OF Integer TO Integer = []\n    pool = collections::append(pool, thread::start(work, 1))\n    total = total + len(pool)\n",
        ),
        (
            "b695_thread_record",
            "    LET h AS T = T[t := thread::start(work, 1)]\n    total = total + thread::waitFor(h.t)\n",
        ),
    ];
    for (name, body) in thread_shapes {
        let source = format!(
            "IMPORT collections\nIMPORT io\nIMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer\n  RETURN n\nEND FUNC\n\
TYPE T\n  t AS RES Thread OF Integer TO Integer\nEND TYPE\n\
FUNC main() AS Nothing\n  MUT total AS Integer = 0\n  MUT i AS Integer = 0\n  WHILE i < {{N}}\n{body}    i = i + 1\n  END WHILE\n  io::print(\"total=\" & toString(total))\nEND FUNC\n"
        );
        let grew = thread_loop_growth(name, &source, "total=50", "total=100");
        assert_eq!(grew, 0, "{name}: main-arena live_bytes grew {grew} B");
    }
}
