//! plan-176-B: `rt_debug_soak`'s resource-ownership, trap/state, thread and record-free
//! soak cases. The shared bounds and helpers live in the root, `rt_debug_soak.rs`.

use super::*;

/// bug-623 residual: a resource union bound straight from a producer frees the variant
/// record it owns (96 B per bind after the box fix: only the 16 B box was freed).
#[test]
fn a_resource_union_bound_from_a_producer_keeps_live_bytes_constant() {
    assert_live_bytes_within(
        "soak_union_direct",
        300,
        600,
        BLOCK_BOUND,
        "a resource union bound from a producer leaks its variant record (bug-623)",
        |n| {
            common::temp_project(
                "soak_union_direct",
                &format!(
                    "IMPORT io\nIMPORT udp\nIMPORT fs\n\nUNION Chan\n  udp::Socket\n  fs::File\nEND UNION\n\nFUNC main() AS Nothing\n  FOR i = 1 TO {n}\n    RES c AS Chan = udp::bind(\"127.0.0.1\", 0)\n  NEXT\n  io::print(\"done\")\nEND FUNC\n"
                ),
            )
        },
    );
}

/// bug-623 guard: a resource union aliasing a live concrete binding frees nothing the
/// concrete binding still owns — the record is freed once, by `u`'s drop.
#[test]
fn a_resource_union_aliasing_a_binding_keeps_live_bytes_constant() {
    assert_live_bytes_within(
        "soak_union_alias",
        300,
        600,
        BLOCK_BOUND,
        "a resource union alias leaks or double-frees (bug-623)",
        |n| {
            common::temp_project(
                "soak_union_alias",
                &format!(
                    "IMPORT io\nIMPORT udp\nIMPORT fs\n\nUNION Chan\n  udp::Socket\n  fs::File\nEND UNION\n\nFUNC main() AS Nothing\n  FOR i = 1 TO {n}\n    RES u AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n    RES c AS Chan = u\n  NEXT\n  io::print(\"done\")\nEND FUNC\n"
                ),
            )
        },
    );
}
/// bug-623 regression (introduced by the record free, fixed by the ownership pass): a
/// function that returns its resource parameter hands the caller back the caller's own
/// record, and the caller's second binding must not free it again. Before the ownership
/// pass: `free_calls` 752 against `alloc_calls` 600 and `double_free_skips` 152 at N=300.
#[test]
fn a_resource_passed_through_a_function_is_freed_once() {
    const SOURCE: &str = "IMPORT io\nIMPORT udp\n\nFUNC passthru(u AS udp::Socket) AS RES udp::Socket\n  RETURN u\nEND FUNC\n\nFUNC main() AS Nothing\n  FOR i = 1 TO {n}\n    RES a AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n    RES b AS udp::Socket = passthru(a)\n  NEXT\n  io::print(\"done\")\nEND FUNC\n";
    for n in [300u64, 600] {
        let (_, (allocs, frees, _, skips)) = debug_run("res_passthru", SOURCE, n);
        assert!(
            skips == 0 && frees <= allocs,
            "res_passthru N={n}: double_free_skips {skips}, free_calls {frees} > alloc_calls \
             {allocs} — a passed-through record was freed twice (bug-623)"
        );
    }
    let (_, (_, _, at_small, _)) = debug_run("res_passthru_flat", SOURCE, 300);
    let (_, (_, _, at_large, _)) = debug_run("res_passthru_flat", SOURCE, 600);
    assert!(
        at_large.saturating_sub(at_small) < BLOCK_BOUND,
        "res_passthru: live_bytes grew {at_small} -> {at_large} between 300 and 600 (bug-623)"
    );
}

/// Found by bug-623 (pre-existing; a use-after-free since the record free): a function that
/// returns a resource union wrapping its OWN owned local handed the caller a closed handle
/// — `udp::localAddress` on it raised `7-703-0004` and the program exited 255 — because the
/// callee's scope drop closed (and now freed) the record the union still pointed at.
#[test]
fn a_returned_union_wrapping_an_owned_local_stays_open_in_the_caller() {
    const SOURCE: &str = "IMPORT io\nIMPORT net\nIMPORT udp\nIMPORT fs\n\nUNION Chan\n  udp::Socket\n  fs::File\nEND UNION\n\nFUNC open() AS RES Chan\n  RES u AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n  RES c AS Chan = u\n  RETURN c\nEND FUNC\n\nFUNC portOf(c AS Chan) AS Integer\n  MUT port AS Integer = -1\n  MATCH c\n    CASE udp::Socket(s)\n      LET a AS net::Address = udp::localAddress(s)\n      port = a.port\n    CASE fs::File(f)\n      port = -2\n  END MATCH\n  RETURN port\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    RES c AS Chan = open()\n    IF portOf(c) > 0 THEN\n      ok = ok + 1\n    END IF\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND FUNC\n";
    let (stdout_small, (_, _, at_small, skips_small)) = debug_run("ret_union_owned", SOURCE, 50);
    let (stdout_large, (_, _, at_large, skips_large)) = debug_run("ret_union_owned", SOURCE, 100);
    assert!(
        stdout_small.contains("ok=50") && stdout_large.contains("ok=100"),
        "every returned union must still be an open socket:\n{stdout_small}\n{stdout_large}"
    );
    assert!(
        skips_small == 0 && skips_large == 0,
        "ret_union_owned: double_free_skips {skips_small}/{skips_large}"
    );
    assert!(
        at_large.saturating_sub(at_small) < BLOCK_BOUND,
        "ret_union_owned: live_bytes grew {at_small} -> {at_large} between 50 and 100"
    );
}

/// Found by bug-623 (pre-existing, same class): a resource union aliasing an outer binding
/// in an inner scope closed the OUTER handle when the inner scope ended — the union
/// registered its own close. `udp::localAddress(u)` afterwards raised `7-703-0004`.
#[test]
fn a_union_alias_in_an_inner_scope_leaves_the_outer_handle_open() {
    const SOURCE: &str = "IMPORT io\nIMPORT net\nIMPORT udp\nIMPORT fs\n\nUNION Chan\n  udp::Socket\n  fs::File\nEND UNION\n\nFUNC main() AS Nothing\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    RES u AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n    MUT flag AS Boolean = 1 > 0\n    IF flag THEN\n      RES c AS Chan = u\n    END IF\n    LET a AS net::Address = udp::localAddress(u)\n    IF a.port > 0 THEN\n      ok = ok + 1\n    END IF\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND FUNC\n";
    let (stdout_small, (_, _, at_small, skips_small)) = debug_run("union_scope_alias", SOURCE, 50);
    let (stdout_large, (_, _, at_large, skips_large)) = debug_run("union_scope_alias", SOURCE, 100);
    assert!(
        stdout_small.contains("ok=50") && stdout_large.contains("ok=100"),
        "the outer handle must stay open after the inner alias's scope ends:\n{stdout_small}\n{stdout_large}"
    );
    assert!(
        skips_small == 0 && skips_large == 0,
        "union_scope_alias: double_free_skips {skips_small}/{skips_large}"
    );
    assert!(
        at_large.saturating_sub(at_small) < BLOCK_BOUND,
        "union_scope_alias: live_bytes grew {at_small} -> {at_large} between 50 and 100"
    );
}
/// Run `exe` with `RLIMIT_NOFILE` capped at `limit`, returning (exit success, stdout+stderr).
fn run_with_fd_limit(exe: &Path, limit: u64) -> (bool, String) {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(exe);
    unsafe {
        command.pre_exec(move || {
            let rl = libc::rlimit {
                rlim_cur: limit as libc::rlim_t,
                rlim_max: limit as libc::rlim_t,
            };
            if libc::setrlimit(libc::RLIMIT_NOFILE, &rl) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = command.output().expect("run the program under an fd limit");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

/// Found by bug-623's union-alias fix (reading `emit_return_exit_inner`, then measured): a
/// function with two owned sockets and two `RETURN`s — `IF give THEN RETURN a END IF` /
/// `RETURN b` — removed `a`'s close permanently while lowering the first `RETURN`, so the
/// fall-through path that returns `b` never closed `a`. One socket leaked per such call:
/// under a 128-descriptor limit `udp::bind` failed after ~250 calls (`7-707-0003`, exit 255),
/// and `live_bytes` grew 96 B per two calls.
#[test]
fn a_resource_not_returned_on_a_sibling_path_is_still_closed() {
    const SOURCE: &str = "IMPORT io\nIMPORT net\nIMPORT udp\n\nFUNC pick(give AS Boolean) AS RES udp::Socket\n  RES a AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n  RES b AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n  IF give THEN\n    RETURN a\n  END IF\n  RETURN b\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    RES s AS udp::Socket = pick((i MOD 2) = 0)\n    LET addr AS net::Address = udp::localAddress(s)\n    IF addr.port > 0 THEN\n      ok = ok + 1\n    END IF\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND FUNC\n";
    let project = common::temp_project("ret_sibling", &SOURCE.replace("{n}", "600"));
    let exe = build_debug_project("ret_sibling_600", &project);
    let (ok, output) = run_with_fd_limit(&exe, 128);
    assert!(
        ok && output.contains("ok=600"),
        "600 calls under a 128-descriptor limit must succeed — the socket a sibling RETURN \
         path does not return must be closed:\n{output}"
    );
    let (_, (_, _, at_small, skips_small)) = debug_run("ret_sibling_flat", SOURCE, 300);
    let (_, (_, _, at_large, skips_large)) = debug_run("ret_sibling_flat", SOURCE, 600);
    assert!(
        skips_small == 0 && skips_large == 0,
        "ret_sibling: double_free_skips {skips_small}/{skips_large}"
    );
    assert!(
        at_large.saturating_sub(at_small) < BLOCK_BOUND,
        "ret_sibling: live_bytes grew {at_small} -> {at_large} between 300 and 600"
    );
}
/// bug-643: `RES c AS Chan = udp::bind(...) TRAP(e) ... END TRAP` left one 96 B block live
/// per bind even though the `TRAP` never fired. Measured at `1963472c6`: N=100
/// `alloc_calls 504`, `free_calls 404`, `live_bytes 9600`; N=200 `1004`/`804`,
/// `live_bytes 19200` — one resource record per call, `double_free_skips 0`.
///
/// The desugar emits `bind $trap_valN : T` with no initializer — which materializes a
/// CLOSED DEFAULT resource record — then `$trap_valN = ResultValue($trap_resN)` on the
/// success path and `bind c = local $trap_valN`. The assign overwrites the slot with the
/// producer's record and orphans the default one, which no binding owns thereafter. The
/// sibling `a_trap_bind_whose_producer_fails_keeps_live_bytes_constant` is the control that
/// localizes it: when the producer fails the assign never runs, the slot still holds the
/// default, and the loop is flat. `tcp::listen` works around bug-642.
#[test]
fn a_trap_bound_resource_union_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT udp\nIMPORT tcp\n\nUNION Chan\n  udp::Socket\n  tcp::Socket\nEND UNION\n\nFUNC openOr(bad AS Boolean) AS Integer\n  MUT port AS Integer = 0\n  IF bad THEN\n    port = -1\n  END IF\n  RES c AS Chan = udp::bind(\"127.0.0.1\", port) TRAP(e)\n    RETURN 0\n  END TRAP\n  RETURN 1\nEND FUNC\n\nFUNC main() AS Nothing\n  RES keep AS tcp::Listener = tcp::listen(\"127.0.0.1\", 0)\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    ok = ok + openOr(i MOD 2 = 0)\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND FUNC\n";
    assert_block_flat(
        "b643_union_trap",
        SOURCE,
        100,
        200,
        "ok=",
        "a `TRAP`-bound resource union leaks a 96 B resource record per bind (bug-643)",
    );
}

/// bug-643 widened: the same 96 B per bind on a plain concrete resource — the leak is the
/// inline-`TRAP` desugar's, not the union's. Measured at `1963472c6`: N=100
/// `alloc_calls 1202`, `free_calls 1102`, `live_bytes 9600`; N=200 `2402`/`2202`,
/// `live_bytes 19200`.
#[test]
fn a_trap_bound_concrete_resource_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT net\nIMPORT udp\n\nFUNC risky(bad AS Boolean) AS RES udp::Socket\n  RES keep AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n  RES spare AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n  MUT port AS Integer = 0\n  IF bad THEN\n    port = -1\n  END IF\n  RES tried AS udp::Socket = udp::bind(\"127.0.0.1\", port) TRAP(e)\n    RETURN spare\n  END TRAP\n  IF port = 0 THEN\n    RETURN keep\n  END IF\n  RETURN tried\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    RES s AS udp::Socket = risky((i MOD 2) = 0)\n    LET addr AS net::Address = udp::localAddress(s)\n    IF addr.port > 0 THEN\n      ok = ok + 1\n    END IF\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND FUNC\n";
    assert_block_flat(
        "b643_concrete_trap",
        SOURCE,
        100,
        200,
        "ok=",
        "a `TRAP`-bound concrete resource leaks a 96 B resource record per bind (bug-643)",
    );
}

/// bug-643's control, and what localizes the leak: an inline `TRAP` whose producer ALWAYS
/// fails is already FLAT (measured at `1963472c6`: `ok=0`, `live_bytes 0` at both counts).
///
/// The handler path reclaims everything; only the success path leaks. That is the whole
/// diagnosis: the `$trap_valN` bind materializes a closed default record, and on the
/// success path the `$trap_valN = ResultValue(...)` assign overwrites the slot with the
/// producer's record, orphaning the default — 96 B with no owner left. When the producer
/// fails the slot still holds the default, so the drop reclaims it and nothing leaks.
///
/// Note `udp::bind("127.0.0.1", -1)` SUCCEEDS, so a negative port does not exercise this
/// path — an unresolvable host does. Green before the fix and after; it fails only if a fix
/// reclaims the default record twice, or reclaims one the binding still holds.
#[test]
fn a_trap_bind_whose_producer_fails_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT udp\n\nFUNC f(host AS String) AS Integer\n  RES c AS udp::Socket = udp::bind(host, 0) TRAP(e)\n    RETURN 0\n  END TRAP\n  RETURN 1\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    ok = ok + f(\"300.0.0.1\")\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND FUNC\n";
    assert_block_flat(
        "b643_trap_fires",
        SOURCE,
        100,
        200,
        "ok=0",
        "the handler path of an inline `TRAP` bind must stay flat (bug-643 control)",
    );
}

/// bug-648: an inline `TRAP` over a BORROWED element (`collections::get` of a
/// `List OF RES`) must neither own the element nor orphan its own closed default record.
///
/// The fix makes the `$trap_valN` temp's drop run only while a run-time flag says the slot
/// holds an owned value. Its first measured cut left the temp's closed default record
/// closed but never freed — 96 B per iteration (`live_bytes 29040` at N=300, `57840` at
/// N=600) — because bug-623's record-ownership pass saw a borrowed store and stopped
/// calling the temp the record's owner. `tests/net/rt_inline_trap_borrowed_resource.rs`
/// pins that the element stays open; this pins the bytes.
#[test]
fn a_trapped_borrowed_get_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT udp\nIMPORT collections\n\nFUNC main() AS Nothing\n  RES u AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n  MUT socks AS List OF RES udp::Socket = []\n  socks = collections::append(socks, u)\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    RES g AS udp::Socket = collections::get(socks, 0) TRAP(e)\n      RETURN\n    END TRAP\n    ok = ok + 1\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND FUNC\n";
    assert_block_flat(
        "b648_get_trap",
        SOURCE,
        300,
        600,
        "ok=",
        "a `TRAP` over a borrowed element orphans its temp's default record (bug-648)",
    );
}

/// bug-648, both halves of a temp whose stores disagree: odd iterations `poll` a pending
/// datagram out of the list (borrowed — the flag must stay clear, freeing nothing), even
/// iterations time out and `RECOVER` a fresh socket (owned — its record must be freed at
/// the drop). The first cut measured 192 B per iteration on the recovering path alone.
#[test]
fn a_trapped_borrowed_poll_with_an_owned_recover_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT net\nIMPORT udp\nIMPORT collections\n\nFUNC main() AS Nothing\n  RES r AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n  LET at AS net::Address = udp::localAddress(r)\n  RES s AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n  MUT socks AS List OF RES udp::Socket = []\n  socks = collections::append(socks, r)\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    MUT wait AS Integer = 0\n    IF i MOD 2 = 1 THEN\n      udp::send(s, at, \"p\")\n      wait = 5000\n    END IF\n    MUT recovered AS Boolean = FALSE\n    RES p AS udp::Socket = udp::poll(socks, wait) TRAP(e)\n      recovered = TRUE\n      RECOVER udp::bind(\"127.0.0.1\", 0)\n    END TRAP\n    IF recovered = FALSE THEN\n      LET got = udp::receive(p, 16)\n      ok = ok + 1\n    END IF\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND FUNC\n";
    assert_block_flat(
        "b648_poll_recover_trap",
        SOURCE,
        300,
        600,
        "ok=",
        "a `TRAP` whose stores are part borrowed, part owned leaks or double-frees (bug-648)",
    );
}

/// bug-644: `RES c AS Chan STATE Cur = udp::bind(...)` with a String STATE field assigned
/// after the bind left one 32 B block live per iteration. Measured at `1963472c6`: N=50
/// `alloc_calls 252`, `free_calls 202`, `live_bytes 1600`; N=100 `502`/`402`,
/// `live_bytes 3200`, `double_free_skips 0`.
///
/// Run at 300/600, not the bug doc's 50/100: at 32 B per iteration the doc's counts grow
/// only 1600 B, *under* [`BLOCK_BOUND`], so a test written to them passes while the leak is
/// live. 300/600 grows 9600 B (measured), which the bound catches.
///
/// Bisected on the main thread: dropping the `c.state.note = "seen"` assignment (leaving the
/// inline-scalar `hits` update, which takes `try_inplace_state_scalar_assign`) is flat, so
/// the leak is the whole-record `StateAssign` rebuild — it allocates a new STATE block and
/// republishes it through `RESOURCE_OFFSET_STATE` without reclaiming the block it replaced.
#[test]
fn a_stateful_resource_union_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT udp\nIMPORT fs\n\nUNION Chan\n  udp::Socket\n  fs::File\nEND UNION\n\nTYPE Cur\n  hits AS Integer\n  note AS String\nEND TYPE\n\nFUNC main() AS Nothing\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    RES c AS Chan STATE Cur = udp::bind(\"127.0.0.1\", 0)\n    c.state.hits = c.state.hits + 1\n    c.state.note = \"seen\"\n    ok = ok + c.state.hits\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND FUNC\n";
    assert_block_flat(
        "b644_union_state",
        SOURCE,
        300,
        600,
        "ok=",
        "a stateful resource union leaks the STATE block each whole-record rebuild replaces \
         (bug-644)",
    );
}

/// bug-644 on the concrete shape — the same 32 B per iteration with no union in sight
/// (measured at `1963472c6`: N=50 `alloc_calls 202`, `free_calls 152`, `live_bytes 1600`;
/// N=100 `402`/`302`, `live_bytes 3200`; at the 300/600 this runs, `live_bytes`
/// 9600 -> 19200). The union arm and the concrete arm share the `StateAssign` rebuild, so a
/// fix that only reaches the union path leaves this red.
#[test]
fn a_stateful_concrete_resource_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT udp\n\nTYPE Cur\n  hits AS Integer\n  note AS String\nEND TYPE\n\nFUNC main() AS Nothing\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    RES c AS udp::Socket STATE Cur = udp::bind(\"127.0.0.1\", 0)\n    c.state.hits = c.state.hits + 1\n    c.state.note = \"seen\"\n    ok = ok + c.state.hits\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND FUNC\n";
    assert_block_flat(
        "b644_concrete_state",
        SOURCE,
        300,
        600,
        "ok=",
        "a stateful concrete resource leaks the STATE block each whole-record rebuild \
         replaces (bug-644)",
    );
}

/// bug-644's decline pin: with only the inline-scalar `hits` update, the in-place store
/// path fires, no block is replaced and nothing is freed. Flat before the fix and after —
/// it fails only if a fix frees a STATE block the resource still reads through
/// `RESOURCE_OFFSET_STATE`, which would be a use-after-free, not a leak.
#[test]
fn an_in_place_state_scalar_update_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT udp\nIMPORT fs\n\nUNION Chan\n  udp::Socket\n  fs::File\nEND UNION\n\nTYPE Cur\n  hits AS Integer\n  note AS String\nEND TYPE\n\nFUNC main() AS Nothing\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    RES c AS Chan STATE Cur = udp::bind(\"127.0.0.1\", 0)\n    c.state.hits = c.state.hits + 1\n    ok = ok + c.state.hits\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND FUNC\n";
    assert_block_flat(
        "b644_state_inplace",
        SOURCE,
        300,
        600,
        "ok=",
        "the in-place STATE scalar store must neither leak nor free a live STATE block \
         (bug-644)",
    );
}

/// bug-645: a `List OF RES Chan` that owns floated resource-union handles closes each one at
/// drop but frees none of their memory. Measured at `1963472c6`: N=20 `alloc_calls 302`,
/// `free_calls 102`, `live_bytes 16320`; N=40 `602`/`202`, `live_bytes 32640` — 816 B and
/// 10 blocks per outer iteration over 3 elements, `double_free_skips 0`.
#[test]
fn an_owned_list_of_resource_unions_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT udp\nIMPORT fs\nIMPORT collections\n\nUNION Chan\n  udp::Socket\n  fs::File\nEND UNION\n\nFUNC main() AS Nothing\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n    MUT chans AS List OF RES Chan = []\n    FOR j = 1 TO 3\n      RES c AS Chan = udp::bind(\"127.0.0.1\", 0)\n      chans = collections::append(chans, c)\n    NEXT\n    total = total + len(chans)\n  NEXT\n  io::print(\"total=\" & toString(total))\nEND FUNC\n";
    assert_block_flat(
        "b645_union_list",
        SOURCE,
        20,
        40,
        "total=",
        "an owned List OF RES of resource unions frees none of its elements' memory \
         (bug-645)",
    );
}

/// bug-645 on the concrete shape: measured on the main thread at `1963472c6`, a
/// `List OF RES udp::Socket` of the same 3 floated elements leaks 576 B and 7 blocks per
/// outer iteration (N=20 `alloc_calls 222`, `free_calls 82`, `live_bytes 11520`; N=40
/// `442`/`162`, `live_bytes 23040`). So the drain frees almost nothing for EITHER element
/// kind — the union only adds its box and variant record on top.
#[test]
fn an_owned_list_of_concrete_resources_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT udp\nIMPORT collections\n\nFUNC main() AS Nothing\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n    MUT chans AS List OF RES udp::Socket = []\n    FOR j = 1 TO 3\n      RES c AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n      chans = collections::append(chans, c)\n    NEXT\n    total = total + len(chans)\n  NEXT\n  io::print(\"total=\" & toString(total))\nEND FUNC\n";
    assert_block_flat(
        "b645_concrete_list",
        SOURCE,
        20,
        40,
        "total=",
        "an owned List OF RES of concrete resources frees none of its elements' memory \
         (bug-645)",
    );
}

/// bug-646: every `thread::transfer` of a resource leaves one 96 B block live in the
/// SENDER's arena. Measured at `1963472c6`: N=30 `alloc_calls 392`, `free_calls 362`,
/// `live_bytes 2880`; N=60 `782`/`722`, `live_bytes 5760`, `double_free_skips 0`. The send
/// path deep-copies the record into the sender's own arena for the queue (bug-498), the
/// receiver copies it again at `thread::accept`, and nothing frees the queued copy.
///
/// Run at 100/200, not the doc's 30/60: 30/60 grows 2880 B, *under* [`BLOCK_BOUND`], so a
/// test written to the doc's counts cannot fail. 100/200 grows 9600 B (measured).
#[test]
fn a_thread_resource_transfer_loop_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT udp\nIMPORT thread\n\nISOLATED FUNC worker(t AS ThreadWorker OF RES udp::Socket TO Integer, n AS Integer) AS Integer\n  RES s AS udp::Socket = thread::accept(t, 20000)\n  udp::close(s)\n  RETURN 1\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n    RES c AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n    RES a AS Thread OF RES udp::Socket TO Integer = thread::start(worker, 0)\n    thread::transfer(a, c)\n    total = total + thread::waitFor(a)\n  NEXT\n  io::print(\"total=\" & toString(total))\nEND FUNC\n";
    assert_block_flat(
        "b646_transfer",
        SOURCE,
        100,
        200,
        "total=",
        "thread::transfer leaks the queued copy of the resource record (bug-646)",
    );
}

/// bug-646 on the data plane: a `thread::send` of a String leaks 32 B per send in the
/// sender's arena (measured on the main thread at `1963472c6`: N=30 `alloc_calls 332`,
/// `free_calls 302`, `live_bytes 960`; N=60 `662`/`602`, `live_bytes 1920`; at the 200/400
/// this runs, `live_bytes` 6400 -> 12800). Same queue hand-over, same missing owner for the
/// queued copy — the bug doc's 64 B was an unverified estimate.
#[test]
fn a_thread_string_send_loop_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT thread\n\nISOLATED FUNC worker(t AS ThreadWorker OF String TO Integer, n AS Integer) AS Integer\n  LET m AS String = thread::receive(t, 20000)\n  RETURN len(m)\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n    RES a AS Thread OF String TO Integer = thread::start(worker, 0)\n    thread::send(a, \"hello-world\")\n    total = total + thread::waitFor(a)\n  NEXT\n  io::print(\"total=\" & toString(total))\nEND FUNC\n";
    assert_block_flat(
        "b646_send",
        SOURCE,
        200,
        400,
        "total=",
        "thread::send leaks the queued copy of the message (bug-646)",
    );
}

/// bug-629 Part A / bug-649: `thread::send` of a COMPUTED message leaks the caller's own
/// argument temp — the block the `&` built to pass in — in the sender's arena. The send
/// helper deep-copies the message for the queue (bug-498) and then claims the original out
/// of the statement's temp cleanup (`claim_moved_thread_arg_temp`), a rule written when the
/// original itself crossed the boundary; since bug-498 it never does, so the claim only
/// removes the block's one owner. Measured at `ac0f61964`: N=100 `live_bytes 3200`, N=200
/// `6400` (32 B per send); the same loop with a literal message is flat, which is what
/// isolates this from bug-646's queued copy.
#[test]
fn a_thread_send_of_a_computed_message_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT thread\n\nISOLATED FUNC worker(t AS ThreadWorker OF String TO Integer, n AS Integer) AS Integer\n  LET m AS String = thread::receive(t, 20000)\n  RETURN len(m)\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n    RES a AS Thread OF String TO Integer = thread::start(worker, 0)\n    thread::send(a, \"message-\" & toString(i MOD 10))\n    total = total + thread::waitFor(a)\n  NEXT\n  io::print(\"total=\" & toString(total))\nEND FUNC\n";
    assert_block_flat(
        "b629_send_computed",
        SOURCE,
        200,
        400,
        "total=",
        "thread::send leaks the caller's computed argument temp (bug-629 Part A, bug-649)",
    );
}

/// bug-629's other direction: a worker's `thread::send(w, …)` lowers to `thread.emit` and
/// rides the same claim, so a computed message leaks in the WORKER's arena — which is
/// `arena.1` here (the one worker), not the main arena the other cases read. Measured at
/// `ac0f61964`: N=100 `arena.1.live_bytes 1664`, N=200 `3264` (16 B per send). Run at
/// 400/800 so the 6400 B growth clears [`BLOCK_BOUND`].
#[test]
fn a_worker_send_of_a_computed_message_keeps_live_bytes_constant() {
    const SOURCE: &str = "IMPORT io\nIMPORT thread\n\nISOLATED FUNC worker(w AS ThreadWorker OF String TO Integer, n AS Integer) AS Integer\n  MUT k AS Integer = 0\n  WHILE k < {n}\n    thread::send(w, \"reply-\" & toString(k MOD 10))\n    k = k + 1\n  END WHILE\n  RETURN k\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT total AS Integer = 0\n  RES t AS Thread OF String TO Integer = thread::start(worker, 0)\n  MUT i AS Integer = 0\n  WHILE i < {n}\n    LET m AS String = thread::receive(t, 20000)\n    total = total + len(m)\n    i = i + 1\n  END WHILE\n  total = total + thread::waitFor(t)\n  io::print(\"total=\" & toString(total))\nEND FUNC\n";
    let at = |n: u64| -> (String, u64, u64) {
        let name = format!("b629_emit_computed_{n}");
        let project =
            common::temp_project("b629_emit_computed", &SOURCE.replace("{n}", &n.to_string()));
        let exe = build_debug_project(&name, &project);
        let (stdout, stderr) = run_ok(&name, &exe);
        let lines = arena_lines(&name, &stderr);
        (
            stdout,
            counter(&name, &lines, 1, "live_bytes"),
            counter(&name, &lines, 1, "double_free_skips"),
        )
    };
    let (out_small, at_small, skips_small) = at(400);
    let (out_large, at_large, skips_large) = at(800);
    assert!(
        out_small.contains("total=") && out_large.contains("total="),
        "b629_emit_computed: expected \"total=\" at both counts, got {out_small:?} / {out_large:?}"
    );
    assert!(
        skips_small == 0 && skips_large == 0,
        "b629_emit_computed: double_free_skips {skips_small}/{skips_large} — the worker's \
         argument temp is now freed twice"
    );
    let grew = at_large.saturating_sub(at_small);
    assert!(
        grew < BLOCK_BOUND,
        "b629_emit_computed: worker-arena live_bytes grew {grew} B between 400 and 800 sends \
         ({at_small} -> {at_large}); thread::send from a worker (thread.emit) leaks the \
         caller's computed argument temp (bug-629)"
    );
}

/// bug-647: an `fs::File` binding leaks its 96 B resource record, because
/// `resource_record_freed_at_drop` excluded `fs.File` — "shares the record with the
/// drop's buffer reclaim". That reason is stale: `emit_resource_block_reclaim` frees the
/// two `File` buffers, then the STATE block, and only THEN the record, so the record's
/// pointer words are read before it goes back to the arena. Measured at `3d49a969e`:
/// N=100 `live_bytes 9600`, N=200 `19200` (96 B per bind, one block).
///
/// `/etc/hosts` rather than a written fixture file: this suite is already `#![cfg(unix)]`
/// and the path exists on every unix, which keeps the case a single `assert_block_flat`
/// source string.
#[test]
fn an_fs_file_binding_frees_its_resource_record() {
    const SOURCE: &str = "IMPORT io\nIMPORT fs\n\nFUNC readOne(path AS String) AS Integer\n  RES f AS fs::File = fs::openFile(path)\n  RETURN 1\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    ok = ok + readOne(\"/etc/hosts\")\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND FUNC\n";
    assert_block_flat(
        "b647_plain",
        SOURCE,
        100,
        200,
        "ok=",
        "an fs::File binding never frees its resource record (bug-647)",
    );
}

/// bug-647 under an inline `TRAP`, which leaks a SECOND record on top of the one above:
/// the closed default that the `$trap_valN` error-path binding materializes
/// (`emit_closed_resource_record`), whose assign-time reclaim passed `frees_record: false`
/// for the same stale exclusion. Measured at `3d49a969e`: N=100 `live_bytes 19200`,
/// N=200 `38400` (192 B per bind, two blocks) — exactly twice the non-`TRAP` shape above,
/// which is what identifies the second block as another whole record.
#[test]
fn an_fs_file_bound_through_an_inline_trap_frees_both_records() {
    const SOURCE: &str = "IMPORT io\nIMPORT fs\n\nFUNC readOne(path AS String) AS Integer\n  RES f AS fs::File = fs::openFile(path) TRAP(e)\n    RETURN 0\n  END TRAP\n  RETURN 1\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    ok = ok + readOne(\"/etc/hosts\")\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND FUNC\n";
    assert_block_flat(
        "b647_trap",
        SOURCE,
        100,
        200,
        "ok=",
        "an fs::File bound through an inline TRAP leaks its record and the TRAP's closed \
         default record (bug-647)",
    );
}

/// bug-651: a `RES`-marked collection that nothing is ever appended to leaks its own
/// collection block — 48 B per binding, with no resource involved at all. bug-645
/// registered that block's free only alongside an owned-list drain, and a drain exists
/// only when an element FLOATS into the container, so the empty case reached no branch
/// that owned it. Measured at `20f55f995`: N=100 `live_bytes 4800`, N=200 `9600`, with
/// `free_calls 2` against `alloc_calls 102`/`202` — one block per iteration, never freed.
#[test]
fn an_unfloated_list_of_res_frees_its_collection_block() {
    const SOURCE: &str = "IMPORT io\nIMPORT udp\nIMPORT collections\n\nFUNC main() AS Nothing\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n    MUT xs AS List OF RES udp::Socket = []\n    total = total + len(xs)\n  NEXT\n  io::print(\"total=\" & toString(total))\nEND FUNC\n";
    assert_block_flat(
        "b651_unfloated",
        SOURCE,
        100,
        200,
        "total=",
        "a List OF RES with no floated element never frees its collection block (bug-651)",
    );
}
/// bug-650 case 1: a message still sitting in a queue's ring when the thread is released is
/// never freed. bug-646's reclaim protocol only reaches a block the reader actually dequeued
/// (it parks the PREVIOUS read's block on each read); nothing walks the ring itself at
/// release, so every message a worker never receives leaks. Bounded by the queue's capacity
/// per thread — and unbounded across a loop of threads, which is what this measures.
/// Measured at `3d49a969e`: N=50 `live_bytes 6400`, N=100 `12800` (128 B per iteration —
/// four undelivered 32 B messages).
///
/// The worker must stay RUNNING for the whole of the parent's sends, which is why it parks
/// on a full outbound queue instead of simply returning. A worker that exits first makes
/// every later `thread::send` raise `ErrInterrupted` (`7-705-0009`) — the send helper's
/// documented answer for a COMPLETED destination — and the first version of this case did
/// exactly that: it passed standalone and failed under full-suite load, where the parent
/// loses the race. The park is the same trick `thread_runtime_workers::blockOnOutbound`
/// uses.
#[test]
fn an_undelivered_queued_message_is_freed_when_the_thread_is_released() {
    const SOURCE: &str = "IMPORT io\nIMPORT thread\n\nISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  MUT n AS Integer = 0\n  WHILE n < 100\n    thread::send(w, \"ack\")\n    n = n + 1\n  END WHILE\n  RETURN n\n  TRAP(e)\n    RETURN 0\nEND TRAP\nEND FUNC\n\nFUNC drain(t AS Thread OF String TO Integer) AS Integer\n  LET v AS Integer = thread::waitFor(t)\n  RETURN v\n  TRAP(e)\n    RETURN 0\nEND TRAP\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT total AS Integer = 0\n  MUT i AS Integer = 0\n  WHILE i < {n}\n    RES t AS Thread OF String TO Integer = thread::start(work, \"abc\", 8, 1)\n    thread::send(t, \"message-one\")\n    thread::send(t, \"message-two\")\n    thread::send(t, \"message-three\")\n    thread::send(t, \"message-four\")\n    thread::cancel(t)\n    total = total + drain(t)\n    i = i + 1\n  END WHILE\n  io::print(\"total=\" & toString(total))\nEND FUNC\n";
    assert_block_flat(
        "b650_unread",
        SOURCE,
        50,
        100,
        "total=",
        "a message left undelivered in the ring is never freed (bug-650 case 1)",
    );
}

/// bug-650 case 2: a `thread::transfer` of a stateful resource reclaims neither the queued
/// resource record nor its separate STATE block. The send helper's pending-free entry
/// carries ONE size (arg 3), which cannot describe record + STATE, so
/// `bare_resource_reclaimable` declines for a stateful resource and passes 0 — the
/// fail-safe that skips the reclaim entirely. Measured at `3d49a969e`: N=50
/// `live_bytes 6400`, N=100 `12800` (128 B per transfer — a 96 B record plus a 32 B
/// `Cursor`).
#[test]
fn a_stateful_resource_transfer_frees_its_record_and_its_state() {
    const SOURCE: &str = "IMPORT io\nIMPORT udp\nIMPORT thread\n\nTYPE Cursor\n  pos AS Integer\nEND TYPE\n\nISOLATED FUNC worker(t AS ThreadWorker OF RES udp::Socket STATE Cursor TO Integer, n AS Integer) AS Integer\n  RES s AS udp::Socket STATE Cursor = thread::accept(t, 20000)\n  udp::close(s)\n  RETURN 1\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n    RES c AS udp::Socket STATE Cursor = udp::bind(\"127.0.0.1\", 0)\n    c.state.pos = i\n    RES a AS Thread OF RES udp::Socket STATE Cursor TO Integer = thread::start(worker, 0)\n    thread::transfer(a, c)\n    total = total + thread::waitFor(a)\n  NEXT\n  io::print(\"total=\" & toString(total))\nEND FUNC\n";
    assert_block_flat(
        "b650_state_transfer",
        SOURCE,
        50,
        100,
        "total=",
        "a transferred stateful resource leaks its queued record and STATE block \
         (bug-650 case 2)",
    );
}

/// bug-655: `thread::start` hands the worker its seed block UNCOPIED — it stores the
/// caller's pointer at `THREAD_OFFSET_DATA` and the trampoline passes that same pointer
/// to the entry — so the caller's statement-scope free would be a use-after-free (which
/// is why `claim_moved_thread_arg_temp` claims it), and nothing on the other side ever
/// owned it. One block leaked per started thread. Measured at `f204b84e2`: N=100
/// `live_bytes 1600`, N=200 `3200` (16 B per start), with `free_calls` 100 short of
/// `alloc_calls` at N=100. Runs at 400/800 so the growth clears [`BLOCK_BOUND`].
#[test]
fn a_thread_start_seed_is_freed_when_the_thread_is_released() {
    const SOURCE: &str = "IMPORT io\nIMPORT thread\n\nISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT total AS Integer = 0\n  MUT i AS Integer = 0\n  WHILE i < {n}\n    RES t AS Thread OF String TO Integer = thread::start(work, \"seed-\" & toString(i MOD 10), 4, 4)\n    total = total + thread::waitFor(t)\n    i = i + 1\n  END WHILE\n  io::print(\"total=\" & toString(total))\nEND FUNC\n";
    assert_block_flat(
        "b655_seed",
        SOURCE,
        400,
        800,
        "total=",
        "thread::start's seed block has no owner (bug-655)",
    );
}

/// bug-655's soundness gate, from the crash side. A string LITERAL seed is a static
/// symbol, not arena memory: an unconditional seed free aborts with SIGBUS on the first
/// iteration. The first version of the fix did exactly that, so this pins it — the case
/// asserts the program RUNS as much as that it stays flat.
#[test]
fn a_thread_start_with_a_literal_seed_frees_nothing() {
    const SOURCE: &str = "IMPORT io\nIMPORT thread\n\nISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT total AS Integer = 0\n  MUT i AS Integer = 0\n  WHILE i < {n}\n    RES t AS Thread OF String TO Integer = thread::start(work, \"abc\", 4, 4)\n    total = total + thread::waitFor(t)\n    i = i + 1\n  END WHILE\n  io::print(\"total=\" & toString(total))\nEND FUNC\n";
    assert_block_flat(
        "b655_literal_seed",
        SOURCE,
        100,
        200,
        "total=",
        "a literal seed is a static symbol and must never be freed (bug-655)",
    );
}

/// bug-655's other soundness gate. A `Local` seed is still owned by its binding, whose
/// own scope-drop frees it; handing it to the thread as well is a double free, and the
/// first version of the fix segfaulted here. The seed must be freed by the thread
/// exactly when `claim_moved_thread_arg_temp` stops the caller from freeing it — this
/// case is the half where the caller keeps it.
#[test]
fn a_thread_start_with_a_local_seed_leaves_it_to_its_binding() {
    const SOURCE: &str = "IMPORT io\nIMPORT thread\n\nISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT total AS Integer = 0\n  MUT i AS Integer = 0\n  WHILE i < {n}\n    LET s AS String = \"seed-\" & toString(i MOD 10)\n    RES t AS Thread OF String TO Integer = thread::start(work, s, 4, 4)\n    total = total + thread::waitFor(t)\n    total = total + len(s)\n    i = i + 1\n  END WHILE\n  io::print(\"total=\" & toString(total))\nEND FUNC\n";
    assert_block_flat(
        "b655_local_seed",
        SOURCE,
        100,
        200,
        "total=",
        "a Local seed is freed by its own binding, never twice (bug-655)",
    );
}

/// bug-658: a FAILED `thread::start` strands its seed. bug-655 gave the seed an owner on
/// the path where the start SUCCEEDS — the thread frees it at its release — but a failed
/// start has no thread to do that: the inline `TRAP` binds the zeroed CLOSED handle,
/// whose `THREAD_OFFSET_DATA` is 0, and the caller gave up its own claim before the call,
/// because at the point the claim is emitted nobody knows yet whether the start will
/// succeed. Measured at `41a7e2063`: N=50 `live_bytes 800`, N=100 `1600` (16 B per failed
/// start). `inboundLimit` 0 is what makes every start fail.
///
/// Runs at 400/800 so the growth clears [`BLOCK_BOUND`]. The case also pins the two
/// crashes the fix had to avoid: freeing on BOTH outcomes double-frees a successful
/// start's seed, and freeing without parking the result registers across the
/// `arena_free` call hands the trapped binding whatever the free left behind — both
/// measured as SIGSEGV.
#[test]
fn a_failed_thread_start_frees_its_seed() {
    const SOURCE: &str = "IMPORT io\nIMPORT thread\n\nISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  RETURN len(seed)\nEND FUNC\n\nFUNC once(i AS Integer) AS Integer\n  RES t AS Thread OF String TO Integer = thread::start(work, \"seed-\" & toString(i MOD 10), 0, 4) TRAP(e)\n    RETURN 0\n  END TRAP\n  RETURN thread::waitFor(t)\nEND FUNC\n\nFUNC main() AS Nothing\n  MUT total AS Integer = 0\n  MUT i AS Integer = 0\n  WHILE i < {n}\n    total = total + once(i)\n    i = i + 1\n  END WHILE\n  io::print(\"total=\" & toString(total))\nEND FUNC\n";
    assert_block_flat(
        "b658_failed_start",
        SOURCE,
        400,
        800,
        "total=",
        "a failed thread::start strands its seed block (bug-658)",
    );
}
