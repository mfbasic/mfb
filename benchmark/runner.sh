#!/usr/bin/env bash
# Shared benchmark runner. A benchmark's run.sh sources this file, builds its
# implementations, then calls time_run once per implementation.
#
#   source "$(dirname "${BASH_SOURCE[0]}")/../runner.sh"
#   bench_build_mfb "$here/mfb"          # -> $MFB_OUT
#   bench_build_c   "$here/c" mybench    # -> $here/c/mybench-O0.out, -O2.out
#   echo "title:"
#   time_run "mfb"    "$MFB_OUT"
#   time_run "python" python3 "$here/python/main.py"
#   time_run "c -O2"  "$here/c/mybench-O2.out"
#
# Each timed program is run $BENCH_RUNS times (default 1000; override via the
# environment). time_run reports median, average, min (shortest) and max
# (longest). Prefer the median — the average is dragged up by occasional OS
# scheduling outliers, while the median tracks the typical run.

set -euo pipefail

BENCH_RUNS="${BENCH_RUNS:-1000}"

_runner_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_repo_root="$(cd "$_runner_dir/.." && pwd)"
MFB="${MFB:-$_repo_root/target/debug/mfb}"

# Built executables to delete on exit (they are git-ignored, but kept tidy).
BENCH_ARTIFACTS=()
_bench_cleanup() { rm -f "${BENCH_ARTIFACTS[@]:-}"; }
trap _bench_cleanup EXIT

# High-resolution wall-clock seconds since the epoch (macOS `date` lacks %N).
now() { perl -MTime::HiRes=time -e 'printf "%.9f\n", time'; }

# bench_build_mfb DIR — build the MFBASIC project in DIR; sets $MFB_OUT to the
# resulting executable (named after the project's "name" field).
bench_build_mfb() {
  local dir="$1"
  "$MFB" build "$dir" >/dev/null
  local name
  name="$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$dir/project.json" | head -1)"
  MFB_OUT="$dir/$name.out"
  BENCH_ARTIFACTS+=("$MFB_OUT")
}

# bench_build_c DIR NAME — compile DIR/main.c at -O0 and -O2 into
# DIR/NAME-O0.out and DIR/NAME-O2.out.
bench_build_c() {
  local dir="$1" name="$2"
  cc -O0 -o "$dir/$name-O0.out" "$dir/main.c"
  cc -O2 -o "$dir/$name-O2.out" "$dir/main.c"
  BENCH_ARTIFACTS+=("$dir/$name-O0.out" "$dir/$name-O2.out")
}

# time_run LABEL CMD... — run CMD $BENCH_RUNS times (stdout discarded) and print
# median / average / min / max wall time.
time_run() {
  local label="$1"; shift
  local starts="" ends="" i
  for ((i = 0; i < BENCH_RUNS; i++)); do
    starts+="$(now) "
    "$@" >/dev/null
    ends+="$(now) "
  done
  perl -e '
    my @s = grep { length } split /\s+/, $ARGV[1];
    my @e = grep { length } split /\s+/, $ARGV[2];
    my ($min, $max, $sum, @d);
    for my $i (0 .. $#s) {
      my $d = $e[$i] - $s[$i];
      $sum += $d;
      push @d, $d;
      $min = $d if !defined($min) || $d < $min;
      $max = $d if !defined($max) || $d > $max;
    }
    my $n = scalar @s;
    my @sorted = sort { $a <=> $b } @d;
    my $med = $n % 2
      ? $sorted[int($n / 2)]
      : ($sorted[$n / 2 - 1] + $sorted[$n / 2]) / 2;
    printf "%-10s med %9.3f ms   avg %9.3f ms   min %9.3f ms   max %9.3f ms   (%d runs)\n",
      $ARGV[0], $med * 1000, ($sum / $n) * 1000, $min * 1000, $max * 1000, $n;
  ' "$label" "$starts" "$ends"
}
