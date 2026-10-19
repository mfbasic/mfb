//! plan-176-C: `rt_scope_drop_leaks`' runtime-helper result (bug-566), helper scratch
//! (bug-574), error filename depth and raised-error (bug-573) tests. The shared
//! helpers live in the root, `rt_scope_drop_leaks.rs`.

use super::*;

// ---------------------------------------------------------------- bug-566

/// Assert `trapped` grows no faster with its iteration count than `plain` does.
///
/// bug-566's shapes cannot use `assert_flat`: a runtime-helper call with a
/// `String` ARGUMENT leaks ~128 B per call on this compiler whether or not a
/// `TRAP` is anywhere near it (`fs::exists(path)` alone, no `TRAP`, no block
/// result, grows 3.6 -> 6.2 MB at 20k/40k). That is a different defect, filed
/// separately, and it would swamp a flatness assertion here.
///
/// So the pin is COMPARATIVE and it isolates exactly what bug-566 changed: the
/// same call, once under an inline `TRAP` and once not. Before the fix the
/// trapped form grew twice as fast (5.2 MB vs 2.6 MB over the same 20k extra
/// iterations); after it, the two growths match. When the argument leak is fixed
/// both sides go flat and this still holds.
#[cfg(unix)]
fn assert_no_extra_growth(name: &str, trapped: &str, plain: &str, small: u64, large: u64) {
    let trapped_growth =
        peak_rss(name, trapped, large).saturating_sub(peak_rss(name, trapped, small));
    let plain_growth = peak_rss(&format!("{name}_plain"), plain, large).saturating_sub(peak_rss(
        &format!("{name}_plain"),
        plain,
        small,
    ));
    assert!(
        trapped_growth <= plain_growth + 2 * 1024 * 1024,
        "{name}: under an inline `TRAP` the loop grew {} MB between {small} and \
         {large} iterations, but the SAME call bound without a `TRAP` grew only \
         {} MB. The `TRAP` lowering copies the helper's block into the `Result` \
         and must free the original (bug-566)",
        trapped_growth / (1024 * 1024),
        plain_growth / (1024 * 1024),
    );
    // bug-574: these three cases were COMPARATIVE when bug-566 landed, because
    // the plain call they compare against was itself leaking — the marshalled
    // path argument, ~129 B per call for `"b566_probe.txt"`. With that released,
    // both halves are flat, and flatness is the stronger statement: comparative
    // growth cannot tell "both fixed" from "both leaking equally".
    assert!(
        trapped_growth < 8 * 1024 * 1024 && plain_growth < 8 * 1024 * 1024,
        "{name}: the loop grew {} MB trapped / {} MB plain between {small} and \
         {large} iterations. Both forms must now be FLAT: the helper owns the \
         block it returned (bug-566) AND releases the argument it marshalled \
         (bug-574)",
        trapped_growth / (1024 * 1024),
        plain_growth / (1024 * 1024),
    );
}

/// bug-566: `fs::readText` under an inline `TRAP`. The helper allocates the
/// `String` in this thread's arena; `materialize_current_result` copies it into an
/// intermediate, copies that into the `Result`, frees the intermediate (bug-379)
/// and abandoned the original. 6.2 -> 11.4 MB at 20k/40k before, against
/// 3.6 -> 6.2 MB for the same call bound plainly.
const SHAPE_566_TRAPPED_READ_TEXT: &str = "IMPORT io\n\
IMPORT fs\n\
FUNC main() AS Nothing\n\
  fs::writeText(\"b566_probe.txt\", \"hello world!\")\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = fs::readText(\"b566_probe.txt\") TRAP(e)\n\
      RECOVER \"x\"\n\
    END TRAP\n\
    acc = acc + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  fs::deleteFile(\"b566_probe.txt\")\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

/// The same call with the `TRAP` removed — the contrast that attributes the extra
/// growth to the `Result` lowering and not to `fs::readText` itself.
const SHAPE_566_PLAIN_READ_TEXT: &str = "IMPORT io\n\
IMPORT fs\n\
FUNC main() AS Nothing\n\
  fs::writeText(\"b566_probe.txt\", \"hello world!\")\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = fs::readText(\"b566_probe.txt\")\n\
    acc = acc + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  fs::deleteFile(\"b566_probe.txt\")\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

/// A COLLECTION payload on the same path — `List OF Byte`, whose block is freed
/// by the collection drop rather than the `String` one.
const SHAPE_566_TRAPPED_READ_BYTES: &str = "IMPORT io\n\
IMPORT fs\n\
FUNC main() AS Nothing\n\
  fs::writeText(\"b566_probe_b.txt\", \"hello world!\")\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET b AS List OF Byte = fs::readBytes(\"b566_probe_b.txt\") TRAP(e)\n\
      RECOVER []\n\
    END TRAP\n\
    acc = acc + len(b)\n\
    i = i + 1\n\
  END WHILE\n\
  fs::deleteFile(\"b566_probe_b.txt\")\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

const SHAPE_566_PLAIN_READ_BYTES: &str = "IMPORT io\n\
IMPORT fs\n\
FUNC main() AS Nothing\n\
  fs::writeText(\"b566_probe_b.txt\", \"hello world!\")\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET b AS List OF Byte = fs::readBytes(\"b566_probe_b.txt\")\n\
    acc = acc + len(b)\n\
    i = i + 1\n\
  END WHILE\n\
  fs::deleteFile(\"b566_probe_b.txt\")\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

/// A SCALAR payload under the same `TRAP` machinery: `fs::exists` returns a
/// `Boolean`, so `result_payload_is_block` is false and bug-566 emits nothing at
/// all. Its growth is the shared argument leak, and it is identical before and
/// after — the control that says the `TRAP` lowering itself is not what changed.
const SHAPE_566_TRAPPED_SCALAR: &str = "IMPORT io\n\
IMPORT fs\n\
FUNC main() AS Nothing\n\
  fs::writeText(\"b566_probe_s.txt\", \"hello world!\")\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET b AS Boolean = fs::exists(\"b566_probe_s.txt\") TRAP(e)\n\
      RECOVER FALSE\n\
    END TRAP\n\
    IF b THEN\n\
      acc = acc + 1\n\
    END IF\n\
    i = i + 1\n\
  END WHILE\n\
  fs::deleteFile(\"b566_probe_s.txt\")\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

const SHAPE_566_PLAIN_SCALAR: &str = "IMPORT io\n\
IMPORT fs\n\
FUNC main() AS Nothing\n\
  fs::writeText(\"b566_probe_s.txt\", \"hello world!\")\n\
  MUT i AS Integer = 0\n\
  MUT acc AS Integer = 0\n\
  WHILE i < {N}\n\
    LET b AS Boolean = fs::exists(\"b566_probe_s.txt\")\n\
    IF b THEN\n\
      acc = acc + 1\n\
    END IF\n\
    i = i + 1\n\
  END WHILE\n\
  fs::deleteFile(\"b566_probe_s.txt\")\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

#[cfg(unix)]
#[test]
fn a_trapped_runtime_helper_string_result_grows_no_faster_than_the_plain_call() {
    assert_no_extra_growth(
        "b566_read_text",
        SHAPE_566_TRAPPED_READ_TEXT,
        SHAPE_566_PLAIN_READ_TEXT,
        20_000,
        40_000,
    );
}

#[cfg(unix)]
#[test]
fn a_trapped_runtime_helper_collection_result_grows_no_faster_than_the_plain_call() {
    assert_no_extra_growth(
        "b566_read_bytes",
        SHAPE_566_TRAPPED_READ_BYTES,
        SHAPE_566_PLAIN_READ_BYTES,
        20_000,
        40_000,
    );
}

/// The scalar control. It held before the fix too — which is the point: the extra
/// growth the two cases above measured was the PAYLOAD BLOCK, not the `TRAP`.
#[cfg(unix)]
#[test]
fn a_trapped_runtime_helper_scalar_result_was_never_the_leak() {
    assert_no_extra_growth(
        "b566_scalar",
        SHAPE_566_TRAPPED_SCALAR,
        SHAPE_566_PLAIN_SCALAR,
        20_000,
        40_000,
    );
}

/// The VALUE half, and the half that matters most here: freeing a block this
/// thread does not own is not a leak fix but memory corruption, and it shows up as
/// a wrong value or an unrelated allocation failure rather than as a failing free.
///
/// `thread::waitFor` is the counter-example the whole audit turns on — its result
/// was allocated in the WORKER's arena — so it is exercised beside the helpers that
/// ARE freed, 25 times, with the values computed here.
#[test]
fn every_trapped_runtime_helper_result_still_produces_the_right_value() {
    const SOURCE: &str = "IMPORT io\n\
IMPORT fs\n\
IMPORT os\n\
IMPORT thread\n\
ISOLATED FUNC worker(w AS ThreadWorker OF String TO String, seed AS String) AS String\n\
  RETURN seed & \"-done\"\n\
END FUNC\n\
FUNC main() AS Nothing\n\
  fs::writeText(\"b566_values.txt\", \"hello world!\")\n\
  LET s AS String = fs::readText(\"b566_values.txt\") TRAP(e)\n\
    RECOVER \"x\"\n\
  END TRAP\n\
  io::print(s)\n\
  LET b AS List OF Byte = fs::readBytes(\"b566_values.txt\") TRAP(e)\n\
    RECOVER []\n\
  END TRAP\n\
  io::print(toString(len(b)))\n\
  LET missing AS String = fs::readText(\"b566_absent_file.txt\") TRAP(e)\n\
    RECOVER \"recovered\"\n\
  END TRAP\n\
  io::print(missing)\n\
  LET names AS List OF String = fs::listDirectory(\".\") TRAP(e)\n\
    RECOVER []\n\
  END TRAP\n\
  io::print(toString(len(names) > 0))\n\
  LET env AS String = os::getEnvOr(\"B566_NOT_SET\", \"fallback\") TRAP(e)\n\
    RECOVER \"x\"\n\
  END TRAP\n\
  io::print(env)\n\
  RES t AS Thread OF String TO String = thread::start(worker, \"worker\")\n\
  LET out AS String = thread::waitFor(t) TRAP(e)\n\
    RECOVER \"thread-failed\"\n\
  END TRAP\n\
  io::print(out)\n\
  io::print(s & \"/\" & missing & \"/\" & env)\n\
  fs::deleteFile(\"b566_values.txt\")\n\
END FUNC\n";

    let expected = [
        "hello world!",
        "12",
        "recovered",
        "TRUE",
        "fallback",
        "worker-done",
        "hello world!/recovered/fallback",
    ]
    .join("\n");

    let project = common::temp_project("b566_values", SOURCE);
    let exe = common::build_project(&project);
    // A cross-arena free is not deterministic: it depends on whether the other
    // arena reuses the block before the read.
    for run in 1..=25 {
        let output = std::process::Command::new(&exe)
            .current_dir(&project)
            .output()
            .expect("run the trapped-runtime-helper ownership probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected,
            "run {run}: a trapped runtime-helper result read back wrong. If it is \
             the `thread::waitFor` line, the free reached a block the WORKER's \
             arena owns — `x19` is per-thread"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-574
//
// Every `fs::`/`os::`/`net::`/`udp::`/`tcp::` call copies its `String` argument
// into a fresh NUL-terminated arena block for the host call. That block is
// interior to the helper — not a `ValueResult` any node yielded — so no
// caller-side ownership analysis could reach it and nothing freed it.
//
// The measurement that identifies it is the LENGTH SCALING, not the absolute
// growth: at 20 000 iterations a 10-character path cost ~65 B per call and a
// 415-character path ~1 819 B, while a zero-argument helper was flat. So the two
// cases below are the same call with the same result type and the same
// iteration counts, differing only in how long the argument is — the long one is
// what a chunk-growth threshold cannot absorb.

/// `fs::exists` with a 10-character path. `Boolean` result, no `TRAP`, no block
/// anywhere in the shape: whatever grows here is the ARGUMENT.
const SHAPE_574_SHORT_PATH: &str = "IMPORT io\n\
IMPORT fs\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    IF fs::exists(\"/tmp/x.txt\") THEN\n\
      n = n + 1\n\
    END IF\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// The same call with a 415-character path — 28x the leak per call, and the
/// sensitive form. A fix that freed the wrong SIZE (bug-560's shape) or freed on
/// only one exit path still reads as growth here.
const SHAPE_574_LONG_PATH: &str = concat!(
    "IMPORT io\n",
    "IMPORT fs\n",
    "FUNC main() AS Nothing\n",
    "  MUT n AS Integer = 0\n",
    "  MUT i AS Integer = 0\n",
    "  WHILE i < {N}\n",
    "    IF fs::exists(\"/tmp/b574probe/",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    ".txt\") THEN\n",
    "      n = n + 1\n",
    "    END IF\n",
    "    i = i + 1\n",
    "  END WHILE\n",
    "  io::print(\"n=\" & toString(n))\n",
    "END FUNC\n",
);

/// The same shape through a LOCAL rather than a literal. The report's own third
/// row: hoisting the path out of the loop changed nothing, which is what ruled
/// out a per-iteration copy of the rodata constant and left the marshalling.
const SHAPE_574_LOCAL_PATH: &str = concat!(
    "IMPORT io\n",
    "IMPORT fs\n",
    "FUNC main() AS Nothing\n",
    "  LET p AS String = \"/tmp/b574probe/",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    ".txt\"\n",
    "  MUT n AS Integer = 0\n",
    "  MUT i AS Integer = 0\n",
    "  WHILE i < {N}\n",
    "    IF fs::exists(p) THEN\n",
    "      n = n + 1\n",
    "    END IF\n",
    "    i = i + 1\n",
    "  END WHILE\n",
    "  io::print(\"n=\" & toString(n))\n",
    "END FUNC\n",
);

/// `os::getEnvOr` with a 400-character variable name that is never set: the
/// `os` family marshals through a different emitter (`marshal_cstring`), and its
/// result is a fresh `String` the binding already owned — so this case separates
/// "the argument is freed" from "the result is freed".
const SHAPE_574_ENV_NAME: &str = concat!(
    "IMPORT io\n",
    "IMPORT os\n",
    "FUNC main() AS Nothing\n",
    "  MUT n AS Integer = 0\n",
    "  MUT i AS Integer = 0\n",
    "  WHILE i < {N}\n",
    "    LET v AS String = os::getEnvOr(\"MFB_B574_",
    "EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE",
    "EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE",
    "EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE",
    "EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE",
    "EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE\", \"d\")\n",
    "    n = n + len(v)\n",
    "    i = i + 1\n",
    "  END WHILE\n",
    "  io::print(\"n=\" & toString(n))\n",
    "END FUNC\n",
);

/// `fs::isWithin` marshals TWO paths and two PATH_MAX `realpath` buffers, and
/// returns a `Boolean` — four scratch blocks and nothing to hand back. The
/// multi-scratch case: a release that covered only the first would still grow.
const SHAPE_574_TWO_ARGUMENTS: &str = "IMPORT io\n\
IMPORT fs\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET b AS Boolean = fs::isWithin(\"/tmp\", \"/tmp\") TRAP(e)\n\
      RECOVER FALSE\n\
    END TRAP\n\
    IF b THEN\n\
      n = n + 1\n\
    END IF\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// `fs::currentDirectory` takes NO argument and still allocated a 4 KiB `getcwd`
/// buffer it never freed — 16 384 B per call, the largest row in the family. It
/// is the case that shows the report's framing — "argument marshalling" — was too
/// narrow: the defect is a fixed runtime helper allocating a block for its own
/// use, whether or not an argument motivated it.
///
/// The result is BOUND deliberately. Left unbound (`len(fs::currentDirectory())`)
/// the same loop still grows ~193 B per call, and it does so identically before
/// and after this change: an UNBOUND runtime-helper `String` result has no owner
/// at all — `os::hostName()` leaks 128 B per call unbound and 0 bound, on both
/// binaries. That is a different defect and is not fixed here; binding keeps this
/// case measuring the scratch buffer alone.
const SHAPE_574_NO_ARGUMENT: &str = "IMPORT io\n\
IMPORT fs\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET here AS String = fs::currentDirectory()\n\
    n = n + len(here)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// The POSITIVE pin: `os::arch()` allocates its result and nothing else, and was
/// flat before this change. It must stay flat — a release that reached the
/// RESULT block would show up here as a wrong value or an allocation failure,
/// not as growth, which is why the value assertions below matter as much.
const SHAPE_574_CONTRAST_ARCH: &str = "IMPORT io\n\
IMPORT os\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET a AS String = os::arch()\n\
    n = n + len(a)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

#[cfg(unix)]
#[test]
fn a_short_path_argument_runs_at_constant_rss() {
    assert_flat("b574_short_path", SHAPE_574_SHORT_PATH, 20_000, 40_000);
}

#[cfg(unix)]
#[test]
fn a_long_path_argument_runs_at_constant_rss() {
    assert_flat("b574_long_path", SHAPE_574_LONG_PATH, 20_000, 40_000);
}

#[cfg(unix)]
#[test]
fn a_long_path_argument_in_a_local_runs_at_constant_rss() {
    assert_flat("b574_local_path", SHAPE_574_LOCAL_PATH, 20_000, 40_000);
}

#[cfg(unix)]
#[test]
fn a_long_environment_name_runs_at_constant_rss() {
    assert_flat("b574_env_name", SHAPE_574_ENV_NAME, 20_000, 40_000);
}

#[cfg(unix)]
#[test]
fn a_call_marshalling_two_arguments_runs_at_constant_rss() {
    assert_flat("b574_two_args", SHAPE_574_TWO_ARGUMENTS, 20_000, 40_000);
}

#[cfg(unix)]
#[test]
fn a_helper_with_no_argument_but_its_own_buffer_runs_at_constant_rss() {
    assert_flat("b574_no_argument", SHAPE_574_NO_ARGUMENT, 20_000, 40_000);
}

#[cfg(unix)]
#[test]
fn a_helper_that_allocates_only_its_result_stays_flat() {
    assert_flat("b574_arch", SHAPE_574_CONTRAST_ARCH, 20_000, 40_000);
}

// ---------------------------------------------------------------- bug-575
//
// `tls` has its own copy of the marshaller (`builtins/tls/gen_shared.rs`), which
// bug-574 did not convert: all twelve of its call sites allocated a NUL-terminated
// arena copy of a host name or a PEM path for the backend call and freed none of
// them. Same defect, same consequence — every `tls::connect` and `tls::listen`
// leaked its host name, in proportion to that name's LENGTH.
//
// ## Why this case is a length comparison and not `assert_flat`
//
// `assert_flat` cannot be used here, and the reason is worth writing down: a
// failing `tls::connect` leaks something else as well. Measured on this shape at
// 20 000/40 000 iterations with a SHORT host, on the already-fixed binary, the
// loop still grows — ~260 B per call on Linux and ~1.9 KB per call on macOS. That
// residue is neither bug-575 nor specific to `tls`: `tcp::connect` in the same
// trapped-failure shape, whose marshalling bug-574 already fixed, grows by the
// same ~260 B per call on the same Linux box. It is the trapped-error path, and it
// is length-INDEPENDENT — identical at a 1 613-character host and a 6 413-character
// one.
//
// So the assertion is the one the bug is actually about: peak RSS must not scale
// with the ARGUMENT's length. Two runs at the same iteration count, differing only
// in how long the host name is, and the difference between them must be noise. The
// length-independent residue is present in both and cancels; the marshalling leak
// does not cancel, because it is 4x larger in the long run.
//
// ## Calibration (measured, `mfb` built from this tree vs. from its parent)
//
// | host chars | 20 000 iterations, peak RSS | before | after |
// | --- | --- | --- | --- |
// | 1 613 | macOS aarch64 (Network.framework) | 173.8 MB | 44.8 MB |
// | 6 413 | macOS aarch64 (Network.framework) | 333.3 MB | 43.5 MB |
// | 1 613 | Linux x86_64 musl (OpenSSL) | 40.4 MB | 8.4 MB |
// | 6 413 | Linux x86_64 musl (OpenSSL) | 160.3 MB | 5.5 MB |
//
// The difference this test measures is therefore +159.5 MB / +119.9 MB before and
// NEGATIVE after (the longer run is the cheaper one once the block is released and
// the arena can reuse it). The threshold is 32 MB: a quarter of the smaller failing
// reading, and ten times the largest passing spread seen.
//
// 20 000 rather than the 200 000 the other cases use, because the leak here is
// ~6 KB per call rather than tens of bytes — the separation at 20 000 is already
// 4x the threshold, and 200 000 would allocate 1.2 GB before failing.
//
// ## What the host name is
//
// A name far past the 253-byte DNS limit, so every resolver rejects it without a
// query — no network, no DNS timeout, no dependence on what the machine's resolver
// does with an unknown name. Both backends marshal the host BEFORE resolving it,
// which is precisely why the leak is reachable through a failing call at all.
//
// Windows is excluded with the rest of the RSS half (`ru_maxrss` has no equivalent
// there), so the Schannel copies of these sites are pinned only by the
// codegen-inspection table in `tests/codegen/codegen_helper_scratch_release.rs`.

/// A loop of `{N}` failing `tls::connect` calls with a host name of
/// `host_chars` characters.
#[cfg(unix)]
fn tls_connect_probe(host_chars: usize) -> String {
    let host = format!("b575-{}.invalid", "z".repeat(host_chars.saturating_sub(13)));
    format!(
        "IMPORT io\n\
         IMPORT tls\n\
         FUNC main() AS Nothing\n\
        \x20 MUT n AS Integer = 0\n\
        \x20 MUT i AS Integer = 0\n\
        \x20 WHILE i < {{N}}\n\
        \x20   RES s AS tls::Socket = tls::connect(\"{host}\", 443, 0) TRAP(e)\n\
        \x20     n = n + 1\n\
        \x20     i = i + 1\n\
        \x20     CONTINUE WHILE\n\
        \x20   END TRAP\n\
        \x20   tls::close(s)\n\
        \x20   i = i + 1\n\
        \x20 END WHILE\n\
        \x20 io::print(\"n=\" & toString(n))\n\
         END FUNC\n"
    )
}

#[cfg(unix)]
#[test]
fn a_tls_connect_host_name_is_not_leaked_in_proportion_to_its_length() {
    const N: u64 = 20_000;
    let short = peak_rss("b575_tls_host_short", &tls_connect_probe(1_613), N);
    let long = peak_rss("b575_tls_host_long", &tls_connect_probe(6_413), N);
    let grew = long.saturating_sub(short);
    assert!(
        grew < 32 * 1024 * 1024,
        "a {N}-iteration `tls::connect` loop cost {} MB more with a 6 413-character \
         host name than with a 1 613-character one ({} MB -> {} MB). The host is \
         copied into an arena C-string for the backend call and that copy is the \
         helper's own scratch — nothing on the caller side can free it, so every \
         call leaks the name (bug-575)",
        grew / (1024 * 1024),
        short / (1024 * 1024),
        long / (1024 * 1024),
    );
}

/// The VALUE half. A scratch release that reached the block a helper HANDS BACK
/// is a use-after-free the caller performs, and it surfaces as a wrong value or a
/// later unrelated allocation failure — never as a failing free. So every member
/// whose emitter this change touched is read back here, in one process, after a
/// warm-up loop that guarantees the arena has recycled the freed scratch into
/// later allocations: if a released block were still live, the reuse would
/// corrupt it before these lines print.
const SHAPE_574_VALUES: &str = "IMPORT io\n\
IMPORT fs\n\
IMPORT os\n\
IMPORT net\n\
IMPORT strings\n\
FUNC main() AS Nothing\n\
  LET d AS String = fs::tempDirectory() & \"/b574_values\"\n\
  fs::createDirectories(d) TRAP(e1)\n\
    RECOVER\n\
  END TRAP\n\
  MUT i AS Integer = 0\n\
  WHILE i < 500\n\
    LET warm AS Boolean = fs::exists(d)\n\
    IF warm THEN\n\
      i = i + 1\n\
    ELSE\n\
      i = i + 1\n\
    END IF\n\
  END WHILE\n\
  fs::writeText(d & \"/a.txt\", \"line one\\nline two\\n\") TRAP(e2)\n\
    RECOVER\n\
  END TRAP\n\
  LET text AS String = fs::readText(d & \"/a.txt\") TRAP(e3)\n\
    RECOVER \"READ FAILED\"\n\
  END TRAP\n\
  io::print(\"text=\" & text)\n\
  LET raw AS List OF Byte = fs::readBytes(d & \"/a.txt\") TRAP(e4)\n\
    RECOVER []\n\
  END TRAP\n\
  io::print(\"bytes=\" & toString(len(raw)))\n\
  io::print(\"exists=\" & toString(fs::exists(d & \"/a.txt\")))\n\
  io::print(\"file=\" & toString(fs::fileExists(d & \"/a.txt\")))\n\
  io::print(\"dir=\" & toString(fs::directoryExists(d)))\n\
  io::print(\"within=\" & toString(fs::isWithin(d, d & \"/a.txt\")))\n\
  LET names AS List OF String = fs::listDirectory(d) TRAP(e5)\n\
    RECOVER []\n\
  END TRAP\n\
  io::print(\"entries=\" & toString(len(names)))\n\
  LET canon AS String = fs::canonicalPath(d & \"/a.txt\") TRAP(e6)\n\
    RECOVER \"CANON FAILED\"\n\
  END TRAP\n\
  io::print(\"canonEndsWithName=\" & toString(strings::endsWith(canon, \"a.txt\")))\n\
  io::print(\"cwd=\" & toString(len(fs::currentDirectory()) > 0))\n\
  io::print(\"tmp=\" & toString(len(fs::tempDirectory()) > 0))\n\
  RES handle AS fs::File = fs::openFile(d & \"/a.txt\", \"r\") TRAP(e7)\n\
    PROPAGATE\n\
  END TRAP\n\
  LET first AS String = fs::readLine(handle) TRAP(e8)\n\
    RECOVER \"LINE FAILED\"\n\
  END TRAP\n\
  io::print(\"line=\" & first)\n\
  fs::close(handle) TRAP(e9)\n\
    RECOVER\n\
  END TRAP\n\
  os::setEnv(\"MFB_B574_VALUE\", \"present\") TRAP(e10)\n\
    RECOVER\n\
  END TRAP\n\
  io::print(\"has=\" & toString(os::hasEnv(\"MFB_B574_VALUE\")))\n\
  io::print(\"env=\" & os::getEnvOr(\"MFB_B574_VALUE\", \"absent\"))\n\
  os::unsetEnv(\"MFB_B574_VALUE\") TRAP(e11)\n\
    RECOVER\n\
  END TRAP\n\
  io::print(\"has2=\" & toString(os::hasEnv(\"MFB_B574_VALUE\")))\n\
  io::print(\"arch=\" & toString(len(os::arch()) > 0))\n\
  LET found AS List OF net::Address = net::lookup(\"localhost\", 80) TRAP(e12)\n\
    RECOVER []\n\
  END TRAP\n\
  io::print(\"lookup=\" & toString(len(found) > 0))\n\
  fs::deleteFile(d & \"/a.txt\") TRAP(e13)\n\
    RECOVER\n\
  END TRAP\n\
  io::print(\"gone=\" & toString(fs::exists(d & \"/a.txt\")))\n\
END FUNC\n";

#[test]
fn every_helper_whose_scratch_is_released_still_produces_the_right_value() {
    let project = common::temp_project("b574_values", SHAPE_574_VALUES);
    let exe = common::build_project(&project);
    let expected = "text=line one\nline two\n\
                    \nbytes=18\n\
                    exists=TRUE\n\
                    file=TRUE\n\
                    dir=TRUE\n\
                    within=TRUE\n\
                    entries=1\n\
                    canonEndsWithName=TRUE\n\
                    cwd=TRUE\n\
                    tmp=TRUE\n\
                    line=line one\n\
                    has=TRUE\n\
                    env=present\n\
                    has2=FALSE\n\
                    arch=TRUE\n\
                    lookup=TRUE\n\
                    gone=FALSE";
    // Ten runs: a released-too-early block is only observably wrong once the
    // arena hands it to a later allocation, and which allocation that is depends
    // on the fill pattern the arena seeds per process.
    for run in 0..10 {
        let output = std::process::Command::new(&exe)
            .current_dir(&project)
            .output()
            .expect("run the helper-scratch value probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected,
            "run {run}: a helper whose marshalling scratch is now released read \
             back wrong. The failure direction for bug-574 is a release that \
             reached the block the helper HANDS BACK, which shows up here rather \
             than as a failing free"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-573
//
// Every error raised through `_mfb_make_error_result` orphaned the `ErrorLoc` it
// had just built: `_mfb_rt_park_error` inlines a COPY of it into the single owned
// `Error` block it parks, and nothing owned the original afterwards, on any path.
//
// The measurement that identifies it as the `ErrorLoc` — rather than anything on
// the `TRAP` side — is that it scales with the recorded SOURCE FILENAME, which is
// inlined into that block and appears nowhere else in the shape. So the two cases
// below are the same program compiled twice, differing only in how deep its
// source file sits: `src/main.mfb` cost ~200 B per raise and a 117-character path
// ~682 B, a slope of 4.6 B per filename byte.

/// Peak RSS of `source` with `{N}` replaced by `count`, compiled at
/// `src/<subdirectory>/main.mfb` so the `ErrorLoc`'s recorded filename is as long
/// as the subdirectory makes it.
#[cfg(unix)]
fn peak_rss_at_depth(name: &str, subdirectory: &str, source: &str, count: u64) -> u64 {
    let program = source.replace("{N}", &count.to_string());
    // `common::temp_project` writes `src/main.mfb`; move the source down into the
    // nested directory (the project's include glob is `**/*.mfb`) so the recorded
    // path grows without changing anything else about the build.
    let project = common::temp_project(&format!("{name}_{count}"), "");
    let nested = project.join("src").join(subdirectory);
    std::fs::create_dir_all(&nested).expect("create nested source directory");
    std::fs::remove_file(project.join("src/main.mfb")).expect("remove the flat source");
    std::fs::write(nested.join("main.mfb"), &program).expect("write nested source");
    let exe = common::build_project(&project);
    let (status, stdout, rss) = common::run_bounded_with_rss(
        &exe,
        std::time::Duration::from_secs(300),
        "the raised-error origin probe did not finish",
    );
    assert!(
        status.success(),
        "{} exited non-zero:\n{stdout}",
        common::exit_description(&status)
    );
    let rss = rss.expect("unix reports ru_maxrss");
    let _ = std::fs::remove_dir_all(&project);
    rss
}

/// Five 20-character path components: `src/<100 chars>/main.mfb` is 117
/// characters against `src/main.mfb`'s 12, so the per-raise cost of the orphaned
/// `ErrorLoc` was 3.4x the flat program's.
#[cfg(unix)]
const DEEP_SOURCE_DIRECTORY: &str = "dddddddddddddddddddd/dddddddddddddddddddd/\
                                     dddddddddddddddddddd/dddddddddddddddddddd/\
                                     dddddddddddddddddddd";

#[cfg(unix)]
#[test]
fn a_raised_error_runs_at_constant_rss_however_long_its_filename_is() {
    let a = peak_rss_at_depth(
        "b573_deep_raise",
        DEEP_SOURCE_DIRECTORY,
        B573_BUILTIN_RAISE,
        200_000,
    );
    let b = peak_rss_at_depth(
        "b573_deep_raise",
        DEEP_SOURCE_DIRECTORY,
        B573_BUILTIN_RAISE,
        400_000,
    );
    let grew = b.saturating_sub(a);
    assert!(
        grew < 8 * 1024 * 1024,
        "b573_deep_raise: peak RSS grew {} MB between 200 000 and 400 000 raised \
         errors ({} MB -> {} MB) with a 117-character source path. The orphaned \
         block is the `ErrorLoc`: its cost is the filename's length, so this is \
         the sensitive form — the flat `src/main.mfb` program leaked 200 B per \
         raise and this one 682 B (137.6 MB -> 274.1 MB before the fix)",
        grew / (1024 * 1024),
        a / (1024 * 1024),
        b / (1024 * 1024),
    );
}

/// The RAISE shape that does not involve an inline builtin at all: a user `FUNC`
/// that `FAIL`s, caught by the caller. Its `ErrorLoc` comes from the same
/// `_mfb_make_error_result` path.
const B573_USER_FAIL: &str = "IMPORT io\n\
FUNC boom(n AS Integer) AS Integer\n\
  FAIL error(90000001, \"boom\")\n\
END FUNC\n\
FUNC main() AS Nothing\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET v AS Integer = boom(i) TRAP(e)\n\
      RECOVER e.code\n\
    END TRAP\n\
    acc = acc + v\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

/// A PROPAGATED error: raised two frames down and re-raised through a
/// `PROPAGATE`, so the middle frame hands the error on rather than raising it.
/// This is the shape the park's re-point protects — the propagation path reads
/// the origin after the raiser has parked, and now reads the parked block's own
/// copy.
const B573_PROPAGATED: &str = "IMPORT io\n\
FUNC boom(n AS Integer) AS Integer\n\
  FAIL error(90000002, \"deep\")\n\
END FUNC\n\
FUNC middle(n AS Integer) AS Integer\n\
  LET v AS Integer = boom(n) TRAP(inner)\n\
    PROPAGATE\n\
  END TRAP\n\
  RETURN v\n\
END FUNC\n\
FUNC main() AS Nothing\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET v AS Integer = middle(i) TRAP(e)\n\
      RECOVER e.source.line\n\
    END TRAP\n\
    acc = acc + v\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

/// A RUNTIME-HELPER error, whose origin is stamped by the call site
/// (`emit_stamp_current_error_source`) rather than by `_mfb_make_error_result` —
/// the second of the three park sites, and the one whose `ErrorLoc` is built
/// directly rather than through the shared assembly.
const B573_HELPER_ERROR: &str = "IMPORT io\n\
IMPORT fs\n\
FUNC main() AS Nothing\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = fs::readText(\"/tmp/b573_absent_file.txt\") TRAP(e)\n\
      RECOVER \"x\"\n\
    END TRAP\n\
    acc = acc + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

/// The POSITIVE pin: the SAME loop with a call that never fails. It was flat
/// before and must stay flat — the change adds an `arena_free` on the error path,
/// so a shape that raises nothing must gain nothing.
const B573_CONTRAST_NO_ERROR: &str = "IMPORT io\n\
FUNC fine(n AS Integer) AS Integer\n\
  RETURN n MOD 3\n\
END FUNC\n\
FUNC main() AS Nothing\n\
  MUT acc AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET v AS Integer = fine(i) TRAP(e)\n\
      RECOVER 0\n\
    END TRAP\n\
    acc = acc + v\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"acc=\" & toString(acc))\n\
END FUNC\n";

#[cfg(unix)]
#[test]
fn a_user_fail_runs_at_constant_rss() {
    assert_flat("b573_user_fail", B573_USER_FAIL, 200_000, 400_000);
}

#[cfg(unix)]
#[test]
fn a_propagated_error_runs_at_constant_rss() {
    assert_flat("b573_propagated", B573_PROPAGATED, 200_000, 400_000);
}

#[cfg(unix)]
#[test]
fn a_runtime_helper_error_runs_at_constant_rss() {
    assert_flat("b573_helper", B573_HELPER_ERROR, 200_000, 400_000);
}

#[cfg(unix)]
#[test]
fn a_loop_that_raises_nothing_still_runs_at_constant_rss() {
    assert_flat("b573_no_error", B573_CONTRAST_NO_ERROR, 200_000, 400_000);
}

/// The VALUE half, and the one that matters: the change frees a block the origin
/// was copied FROM, so its failure direction is a use-after-free that reads back
/// as a wrong filename / line / column, or as an unrelated allocation failure
/// later — never as a failing free.
///
/// Every origin-reading shape is here: an inline builtin's domain error, a
/// runtime helper's stamped origin, a user `FAIL`, a `PROPAGATE` two frames down,
/// a function-level `TRAP`, and the untrapped top-level banner (a separate
/// program, since it ends the process). Each reads `source.filename`,
/// `source.line` and `source.char`, all of which live in the block whose original
/// has just been released.
const B573_ORIGIN_VALUES: &str = "IMPORT io\n\
IMPORT collections\n\
IMPORT fs\n\
FUNC deep(n AS Integer) AS Integer\n\
  IF n = 0 THEN\n\
    FAIL error(90000001, \"from deep\")\n\
  END IF\n\
  RETURN deep(n - 1)\n\
END FUNC\n\
FUNC reraise() AS Integer\n\
  LET v AS Integer = deep(2) TRAP(inner)\n\
    PROPAGATE\n\
  END TRAP\n\
  RETURN v\n\
END FUNC\n\
FUNC viaTrap() AS String\n\
  LET xs AS List OF String = [\"aa\"]\n\
  RETURN collections::get(xs, 9)\n\
  TRAP(err)\n\
    RETURN err.source.filename & \":\" & toString(err.source.line)\n\
  END TRAP\n\
END FUNC\n\
FUNC main() AS Nothing\n\
  LET xs AS List OF String = [\"aa\", \"bb\"]\n\
  MUT i AS Integer = 0\n\
  MUT churn AS Integer = 0\n\
  WHILE i < 500\n\
    LET c AS String = collections::get(xs, 9) TRAP(warm)\n\
      RECOVER warm.source.filename\n\
    END TRAP\n\
    churn = churn + len(c)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"churn=\" & toString(churn))\n\
  LET a AS String = collections::get(xs, 9) TRAP(e1)\n\
    RECOVER e1.source.filename & \":\" & toString(e1.source.line) & \":\" & toString(e1.source.char)\n\
  END TRAP\n\
  io::print(\"builtin=\" & a)\n\
  LET b AS String = fs::readText(\"/tmp/b573_absent_file.txt\") TRAP(e2)\n\
    RECOVER toString(e2.code) & \"@\" & e2.source.filename & \":\" & toString(e2.source.line)\n\
  END TRAP\n\
  io::print(\"helper=\" & b)\n\
  LET c AS Integer = deep(3) TRAP(e3)\n\
    io::print(\"fail=\" & e3.message & \"@\" & e3.source.filename & \":\" & toString(e3.source.line))\n\
    RECOVER 0\n\
  END TRAP\n\
  io::print(\"failGot=\" & toString(c))\n\
  LET d AS Integer = reraise() TRAP(e4)\n\
    io::print(\"prop=\" & toString(e4.code) & \"@\" & e4.source.filename & \":\" & toString(e4.source.line))\n\
    RECOVER 0\n\
  END TRAP\n\
  io::print(\"propGot=\" & toString(d))\n\
  io::print(\"viaTrap=\" & viaTrap())\n\
END FUNC\n";

#[test]
fn every_raised_error_still_reports_its_true_origin() {
    let project = common::temp_project("b573_origins", B573_ORIGIN_VALUES);
    let exe = common::build_project(&project);
    let expected = "churn=6000\n\
                    builtin=src/main.mfb:35:19\n\
                    helper=77030001@src/main.mfb:39\n\
                    fail=from deep@src/main.mfb:6\n\
                    failGot=0\n\
                    prop=90000001@src/main.mfb:6\n\
                    propGot=0\n\
                    viaTrap=src/main.mfb:18";
    // Ten runs: a released-too-early block is only observably wrong once the arena
    // hands it to a later allocation, and which allocation that is depends on the
    // fill pattern the arena seeds per process. The 500-iteration warm-up above
    // guarantees the released `ErrorLoc`s have been recycled before these lines
    // are built.
    for run in 0..10 {
        let output = std::process::Command::new(&exe)
            .current_dir(&project)
            .output()
            .expect("run the raised-error origin probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            expected,
            "run {run}: a raised error reported the wrong origin. bug-573 frees the \
             `ErrorLoc` the parked `Error` block copied, so its failure direction \
             is exactly this — a filename, line or column read out of memory the \
             arena has already handed to something else"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}
