//! bug-760 B2: appending into a `List` value of a `Map` costs what the same
//! appends cost in a `List OF List`.
//!
//! bug-733 made every in-place Map/Set grow call `_mfb_rt_map_reclaim` first
//! (`map_mutate.rs:emit_map_reclaim_before_grow` →
//! `map_bucket_remove.rs:lower_map_reclaim_helper`), which sums every live byte
//! before deciding whether to reclaim, and on a reclaim byte-copies every live
//! payload and invalidates the index. A `List` value that keeps growing relocates
//! and grows the map on nearly every append, so the benchmark's `map listagg`
//! (group ints into a `Map OF Integer TO List OF Integer`) doubled. The fix keeps
//! a running dead-byte count so the check is O(1) and a reclaim runs only when
//! it pays.
//!
//! **The measure.** The program times, in-process, `N` appends spread over `B`
//! buckets two ways — as a `Map OF Integer TO List OF Integer` (`getOr` /
//! `append` / `set`) and as a pre-sized `List OF List OF Integer` (`get` /
//! `append` / `set`) — best of five each. Before bug-733 the map form ran at
//! 0.65–0.79× the list form; with the per-grow scan it runs at 1.85–2.1×. The
//! bound is 1.5×. Both forms fold every bucket into a checksum the test compares,
//! so a reclaim that loses or corrupts a value fails here too.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const N: u64 = 5_000;
const B: u64 = 64;
const MAX_RATIO: f64 = 1.5;

fn program() -> String {
    format!(
        "IMPORT collections\nIMPORT io\nIMPORT datetime\n\n\
         FUNC viaMap() AS Integer\n  \
         MUT m AS Map OF Integer TO List OF Integer = Map OF Integer TO List OF Integer {{}}\n  \
         FOR i = 0 TO {N} - 1\n    \
         LET k AS Integer = i MOD {B}\n    \
         LET empty AS List OF Integer = []\n    \
         MUT bucket AS List OF Integer = collections::getOr(m, k, empty)\n    \
         bucket = collections::append(bucket, i)\n    \
         m = collections::set(m, k, bucket)\n  \
         NEXT\n  \
         MUT acc AS Integer = 0\n  \
         FOR k = 0 TO {B} - 1\n    \
         LET bucket AS List OF Integer = collections::get(m, k)\n    \
         acc = acc + (k + 1) * len(bucket) + collections::sum(bucket) + collections::get(bucket, len(bucket) - 1)\n  \
         NEXT\n  \
         RETURN acc\n\
         END FUNC\n\n\
         FUNC viaLists() AS Integer\n  \
         MUT buckets AS List OF List OF Integer = []\n  \
         FOR k = 0 TO {B} - 1\n    \
         LET empty AS List OF Integer = []\n    \
         buckets = collections::append(buckets, empty)\n  \
         NEXT\n  \
         FOR i = 0 TO {N} - 1\n    \
         LET k AS Integer = i MOD {B}\n    \
         MUT bucket AS List OF Integer = collections::get(buckets, k)\n    \
         bucket = collections::append(bucket, i)\n    \
         buckets = collections::set(buckets, k, bucket)\n  \
         NEXT\n  \
         MUT acc AS Integer = 0\n  \
         FOR k = 0 TO {B} - 1\n    \
         LET bucket AS List OF Integer = collections::get(buckets, k)\n    \
         acc = acc + (k + 1) * len(bucket) + collections::sum(bucket) + collections::get(bucket, len(bucket) - 1)\n  \
         NEXT\n  \
         RETURN acc\n\
         END FUNC\n\n\
         FUNC main AS Integer\n  \
         MUT mapNs AS Integer = 9223372036854775807\n  \
         MUT listNs AS Integer = 9223372036854775807\n  \
         MUT mapSum AS Integer = 0\n  \
         MUT listSum AS Integer = 0\n  \
         FOR r = 0 TO 4\n    \
         LET t0 AS Integer = datetime::monotonicNanos()\n    \
         mapSum = viaMap()\n    \
         LET t1 AS Integer = datetime::monotonicNanos()\n    \
         listSum = viaLists()\n    \
         LET t2 AS Integer = datetime::monotonicNanos()\n    \
         IF t1 - t0 < mapNs THEN\n      mapNs = t1 - t0\n    END IF\n    \
         IF t2 - t1 < listNs THEN\n      listNs = t2 - t1\n    END IF\n  \
         NEXT\n  \
         io::print(toString(mapSum) & \" \" & toString(listSum))\n  \
         io::print(toString(mapNs) & \" \" & toString(listNs))\n  \
         RETURN 0\n\
         END FUNC\n"
    )
}

#[test]
fn appending_into_a_map_list_value_costs_what_a_list_of_lists_costs() {
    let project = common::temp_project("b760_map_list_value_grow", &program());
    let exe = common::build_project(&project);
    let output = Command::new(&exe).output().expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "program failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut lines = stdout.lines();
    let sums: Vec<&str> = lines.next().expect("checksum line").split_whitespace().collect();
    assert_eq!(
        sums[0], sums[1],
        "bug-760 B2: the Map form and the List form disagree — a reclaim lost or corrupted a \
         value:\n{stdout}"
    );
    let times: Vec<f64> = lines
        .next()
        .expect("timing line")
        .split_whitespace()
        .map(|t| t.parse().expect("ns"))
        .collect();
    let ratio = times[0] / times[1];
    eprintln!(
        "N={N} B={B}: map {:.2} ms, list of lists {:.2} ms, ×{ratio:.2}",
        times[0] / 1e6,
        times[1] / 1e6
    );
    assert!(
        ratio <= MAX_RATIO,
        "bug-760 B2: {N} appends into {B} `Map` list values took {:.2} ms against {:.2} ms for \
         the same appends into a `List OF List`: ×{ratio:.2} (bound ×{MAX_RATIO}). Every \
         in-place grow is scanning the whole map before it grows.",
        times[0] / 1e6,
        times[1] / 1e6
    );
    let _ = std::fs::remove_dir_all(&project);
}
