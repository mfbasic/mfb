# remote-common.sh — shared helpers for the remote / app runtime proofs (sourced, not
# executable). plan-131-D.
#
# Sourced by test-winprocess.sh, test-appimage.sh, linux-runtime-proof.sh, test-winapp.sh,
# test-canvas-vulkan.sh and test-macapp.sh:
#
#     . "$(dirname "$0")/remote-common.sh"
#
# What lives here, and why each is shared:
#   pass / fail / rc_failures   one counter and one output format. `pass` prints `ok: …` on
#                               stdout. `fail` prints `FAIL: …` on stdout, or on stderr when
#                               the sourcing script sets RC_FAIL_TO_STDERR=1 first (the
#                               appimage and macapp proofs always reported failures there).
#   rc_workdir                  a mktemp -d work dir, removed on exit.
#   rc_parse_box                `--box <port>` parsing into PORT, erroring on anything else.
#   RC_SSH_OPTS / RC_SCP_OPTS   BatchMode + ConnectTimeout, so a down box or a password
#   remote_ssh / remote_scp     prompt fails fast instead of wedging the script.
#                               MFB_SSH_CONNECT_TIMEOUT overrides the 10 s timeout.
#   watchdog <secs> cmd…        runs cmd, prints its stdout, exits 99 if it outlives <secs>.
#   win_ship <port> <host> <remote-dir> <file>…
#                               mkdir the Windows dir and scp each file into it.
#   scaffold_project <dir> <name>
#                               the standard one-source-root executable project.json.

rc_failures=0
RC_FAIL_TO_STDERR=${RC_FAIL_TO_STDERR:-0}

pass() { echo "ok: $1"; }
fail() {
  if [ "$RC_FAIL_TO_STDERR" = 1 ]; then
    echo "FAIL: $1" >&2
  else
    echo "FAIL: $1"
  fi
  rc_failures=$((rc_failures + 1))
}

# usage: work="$(rc_workdir)" is NOT enough (the trap would live in a subshell); call
# `rc_workdir` directly — it sets $work and installs the cleanup trap.
rc_workdir() {
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
}

# usage: rc_parse_box <script-name> "$@"  — sets PORT from `--box <port>`.
rc_parse_box() {
  local name=$1
  shift
  while [ $# -gt 0 ]; do
    case "$1" in
      --box) PORT="$2"; shift 2 ;;
      *) echo "$name: unknown argument $1" >&2; exit 2 ;;
    esac
  done
}

RC_SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=${MFB_SSH_CONNECT_TIMEOUT:-10}"
RC_SCP_OPTS="$RC_SSH_OPTS"

# usage: remote_ssh <port> <host> [ssh args / command…]
remote_ssh() {
  local port=$1
  shift
  # shellcheck disable=SC2086
  ssh $RC_SSH_OPTS -p "$port" "$@"
}

# usage: remote_scp <port> <scp args…>
remote_scp() {
  local port=$1
  shift
  # shellcheck disable=SC2086
  scp $RC_SCP_OPTS -P "$port" "$@"
}

# usage: watchdog <secs> <cmd> [args…] — prints the command's stdout; exit code is the
# command's, or 99 if it ran past <secs> (98 if it could not be started).
watchdog() {
  local limit=$1
  shift
  perl -e '
    my $limit = shift @ARGV;
    my $pid = open(my $fh, "-|");
    if (!defined $pid) { exit 98; }
    if ($pid == 0) { exec(@ARGV) or exit 127; }
    local $SIG{ALRM} = sub { kill "KILL", $pid; waitpid($pid, 0); exit 99; };
    alarm $limit;
    local $/; my $out = <$fh>; close($fh); my $st = $?;
    print $out if defined $out;
    exit($st >> 8);
  ' "$limit" "$@"
}

# usage: win_ship <port> <host> <remote-dir (Windows form, e.g. C:\mfbwin)> <file>…
# Files land as C:/<dir>/<basename>.
win_ship() {
  local port=$1 host=$2 dir=$3
  shift 3
  remote_ssh "$port" "$host" "if not exist $dir mkdir $dir" >/dev/null
  local fwd
  fwd=$(printf '%s' "$dir" | tr '\\' '/')
  local f
  for f in "$@"; do
    remote_scp "$port" "$f" "$host:$fwd/$(basename "$f")" >/dev/null
  done
}

# usage: scaffold_project <dir> <name> — writes <dir>/project.json and creates <dir>/src.
scaffold_project() {
  mkdir -p "$1/src"
  cat > "$1/project.json" <<JSON
{ "name": "$2", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
}
