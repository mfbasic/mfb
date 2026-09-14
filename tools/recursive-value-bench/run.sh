#!/usr/bin/env bash
# plan-134: build each recursive-value probe and print one row per run:
#
#   name size maxrss_bytes real_s exit stdout
#
# usage: run.sh <mfb> [program...]     (no program names = all of them)
#
# Peak RSS and wall time come from /usr/bin/time (-l on macOS, -v on Linux). Where
# /usr/bin/time is missing, the program is built with --debug instead and RSS is read
# from its report's process.peak_rss_bytes (a crashed run prints no report: "-").
# node_copies additionally prints the deep-copy / alloc / free calls _mfb_fn_main makes,
# counted from its -ncode relocation table.
set -euo pipefail

if [ $# -lt 1 ]; then
  echo "usage: $0 <mfb> [program...]" >&2
  exit 2
fi

here="$(cd "$(dirname "$0")" && pwd)"
mfb="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
shift

# Every probe is built inside this tree (`programs/<name>/build`, and `node_copies`
# writes its `-ncode` dump beside its project), so the run takes the same per-tree lock as
# artifact-gate.sh / test-accept.sh (bug-470). A run in a different worktree is not
# blocked.
GATE_LOCK_HOLDER="recursive-value-bench/run.sh"
GATE_LOCK_TREE="$(cd "$here/../.." && pwd)"
# shellcheck source=../../scripts/gate-lock.sh
. "$here/../../scripts/gate-lock.sh"
gate_lock_acquire || exit $?

all_programs="c_union_rss c_record_rss json_repeat regex_repeat json_get node_copies tree_alias deep_chain deep_build_only regex_chain"
programs="${*:-$all_programs}"

# The sizes each program runs at (its first argument). "-" = no argument.
sizes_for() {
  case "$1" in
    c_union_rss|c_record_rss) echo "400000 800000" ;;
    json_repeat|regex_repeat) echo "1 2 4" ;;
    json_get) echo "1 10 100" ;;
    node_copies|tree_alias) echo "-" ;;
    deep_chain) echo "50000 70000 100000 1000000" ;;
    deep_build_only) echo "1000000" ;;
    regex_chain) echo "simple:1 simple:10000 group:1 group:10000 group:499999 group:500001" ;;
    *) echo "unknown program: $1" >&2; exit 2 ;;
  esac
}

case "$(uname -s)" in
  Darwin) time_flag="-l" ;;
  *) time_flag="-v" ;;
esac
have_time=0
if [ -x /usr/bin/time ]; then have_time=1; fi

build() {
  # $1 = program dir, rest = extra build flags. Prints the executable path.
  local dir="$1"
  shift
  local out exe log
  log="$(mktemp)"
  if ! out="$("$mfb" build "$@" "$dir" 2> "$log")"; then
    echo "build of $dir failed:" >&2
    cat "$log" >&2
    rm -f "$log"
    exit 1
  fi
  rm -f "$log"
  # Linux console builds write one executable per libc; run the glibc one.
  exe="$(printf '%s\n' "$out" | sed -n 's/^Wrote executable to //p' | grep -m1 -- '-glibc' || true)"
  if [ -z "$exe" ]; then
    exe="$(printf '%s\n' "$out" | sed -n 's/^Wrote executable to //p' | head -n1)"
  fi
  if [ -z "$exe" ]; then
    echo "build of $dir wrote no executable:" >&2
    printf '%s\n' "$out" >&2
    exit 1
  fi
  case "$exe" in
    /*) printf '%s\n' "$exe" ;;
    *) printf '%s\n' "$PWD/$exe" ;;
  esac
}

printf 'name size maxrss_bytes real_s exit stdout\n'

for name in $programs; do
  dir="$here/programs/$name"
  if [ ! -f "$dir/project.json" ]; then
    echo "no such program: $name" >&2
    exit 2
  fi
  if [ "$have_time" = 1 ]; then
    exe="$(build "$dir")"
  else
    exe="$(build "$dir" --debug)"
  fi
  for size in $(sizes_for "$name"); do
    args=()
    if [ "$size" != "-" ]; then
      # "a:b" passes two arguments.
      IFS=':' read -r -a args <<< "$size"
    fi
    stdout_file="$(mktemp)"
    stderr_file="$(mktemp)"
    code=0
    if [ "$have_time" = 1 ]; then
      /usr/bin/time "$time_flag" "$exe" ${args[@]+"${args[@]}"} > "$stdout_file" 2> "$stderr_file" || code=$?
      if [ "$time_flag" = "-l" ]; then
        rss="$(awk '/maximum resident set size/ { print $1 }' "$stderr_file" | tail -n1)"
        real="$(awk '/ real / { print $1 }' "$stderr_file" | tail -n1)"
      else
        rss="$(awk -F': ' '/Maximum resident set size/ { print $2 * 1024 }' "$stderr_file" | tail -n1)"
        real="$(awk -F': ' '/Elapsed \(wall clock\)/ { n = split($2, t, ":"); s = 0; for (i = 1; i <= n; i++) s = s * 60 + t[i]; printf "%.2f", s }' "$stderr_file" | tail -n1)"
      fi
      # A signal death: macOS /usr/bin/time re-raises the child's signal on itself (the
      # shell already reports 128+N); GNU time exits 1 and names the signal.
      sig="$(grep -o 'Command terminated by signal [0-9]*' "$stderr_file" | awk '{ print $5 }' | head -n1 || true)"
      if [ -n "$sig" ]; then
        code=$((128 + sig))
      fi
    else
      start="$(perl -MTime::HiRes=time -e 'printf "%.6f", time')"
      "$exe" ${args[@]+"${args[@]}"} > "$stdout_file" 2> "$stderr_file" || code=$?
      end="$(perl -MTime::HiRes=time -e 'printf "%.6f", time')"
      real="$(awk -v a="$start" -v b="$end" 'BEGIN { printf "%.2f", b - a }')"
      rss="$(awk '/^process.peak_rss_bytes / { print $2 }' "$stderr_file" | tail -n1)"
    fi
    out="$(tr '\n' ' ' < "$stdout_file" | sed 's/ *$//; s/ /_/g')"
    printf '%s %s %s %s %s %s\n' "$name" "$size" "${rss:--}" "${real:--}" "$code" "${out:--}"
    rm -f "$stdout_file" "$stderr_file"
  done
  if [ "$name" = node_copies ]; then
    "$mfb" build -ncode "$dir" > /dev/null 2>&1
    ncode="$dir/node_copies.ncode"
    copies="$(grep -c '"from": "_mfb_fn_main", "to": "_mfb_thread_copy_[^"]*", "kind": "branch26"' "$ncode" || true)"
    walker="$(grep -c '"from": "_mfb_fn_main", "to": "_mfb_rt_graph_copy", "kind": "branch26"' "$ncode" || true)"
    allocs="$(grep -c '"from": "_mfb_fn_main", "to": "_mfb_arena_alloc", "kind": "branch26"' "$ncode" || true)"
    frees="$(grep -c '"from": "_mfb_fn_main", "to": "_mfb_arena_free", "kind": "branch26"' "$ncode" || true)"
    emitted="$(grep -c '"symbol": "_mfb_thread_copy_' "$ncode" || true)"
    printf 'node_copies-ncode main_copy_calls=%s main_graph_copy_calls=%s main_arena_alloc=%s main_arena_free=%s copy_functions_emitted=%s\n' \
      "$copies" "$walker" "$allocs" "$frees" "$emitted"
  fi
done
