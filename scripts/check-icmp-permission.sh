#!/usr/bin/env bash
# `net::ping` under an ICMP-denied environment (plan-110-F Phase 1).
#
# plan-110-A §C3 draws a hard line: a host that ANSWERS differently is a
# `PingStatus` (Ok / Timeout / Unreachable / TtlExceeded), but an OS that refuses
# to give us an ICMP socket at all is an ERROR — a raise, never a status. The
# distinction matters because a program that only ever matches on `PingStatus`
# would silently read "no reply" where the real answer is "you were not allowed
# to ask".
#
# The acceptance fixtures cannot test that: `rt-behavior/net/func_net_ping_valid`
# runs wherever ICMP happens to be permitted and says so in its own header. This
# check manufactures the denial instead of waiting to meet it.
#
# How the denial is produced, per host:
#
#   Linux  `unshare -Urn` -- a fresh user + network namespace. A new netns starts
#          with `net.ipv4.ping_group_range = 1 0`, an empty range, so
#          `socket(AF_INET, SOCK_DGRAM, IPPROTO_ICMP)` is refused with EACCES for
#          every gid. Rootless, and it touches nothing outside the namespace.
#   macOS  `sandbox-exec` with a profile that denies `network-outbound`. The
#          socket call itself is refused before any packet is sent.
#
# Either mechanism can be absent (user namespaces disabled, sandbox-exec removed).
# That is reported as SKIP with the reason, never as a pass.
#
# Usage: check-icmp-permission.sh <mfb-exe>
set -u

if [ "$#" -lt 1 ]; then
  echo "usage: check-icmp-permission.sh <mfb-exe>" >&2
  exit 2
fi
MFB_EXE=$1

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/src"
cat >"$work/project.json" <<'EOF'
{ "name": "icmp_permission_check", "version": "0.1.0", "mfb": "1.0",
  "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
EOF
# Prints `raised` when ping refuses to answer at all, or `status:<n>` when it
# returns one. Only the first is correct under a denial.
cat >"$work/src/main.mfb" <<'EOF'
IMPORT io
IMPORT net

FUNC probe AS String
  LET r AS net::PingResult = net::ping("127.0.0.1", 1000)
  MATCH r.status
    CASE PingStatus.Ok
      RETURN "status:Ok"
    CASE PingStatus.Timeout
      RETURN "status:Timeout"
    CASE PingStatus.Unreachable
      RETURN "status:Unreachable"
    CASE PingStatus.TtlExceeded
      RETURN "status:TtlExceeded"
  END MATCH
  RETURN "status:?"
  TRAP(err)
    RETURN "raised"
  END TRAP
END FUNC

FUNC main AS Integer
  io::print(probe())
  RETURN 0
END FUNC
EOF

build_output=$("$MFB_EXE" build "$work" 2>&1) || {
  echo "FAIL: build error" >&2; printf '%s\n' "$build_output" >&2; exit 1; }
exe=$(printf '%s\n' "$build_output" | sed -n 's/^Wrote executable to //p' | tail -n 1)

# Sanity: with ICMP permitted the same program must NOT raise. Without this the
# check would pass on a build where ping raises unconditionally.
permitted=$("$exe" 2>&1)
case "$permitted" in
  status:*) ;;
  *) echo "SKIP: ICMP is already denied in this environment (ping returned '$permitted'),"
     echo "      so the permitted/denied contrast this check relies on cannot be drawn here."
     exit 0 ;;
esac
echo "baseline: ICMP permitted here, ping returned '$permitted'"

case "$(uname -s)" in
  Linux)
    command -v unshare >/dev/null 2>&1 || {
      echo "SKIP: no unshare(1); cannot build an ICMP-denied namespace"; exit 0; }
    denied=$(unshare -Urn "$exe" 2>&1) || {
      echo "SKIP: user namespaces unavailable (unshare -Urn failed): $denied"; exit 0; }
    ;;
  Darwin)
    command -v sandbox-exec >/dev/null 2>&1 || {
      echo "SKIP: no sandbox-exec(1); cannot deny ICMP"; exit 0; }
    profile="$work/deny-net.sb"
    cat >"$profile" <<'SB'
(version 1)
(allow default)
(deny network-outbound)
(deny network-inbound)
SB
    denied=$(sandbox-exec -f "$profile" "$exe" 2>&1) || {
      echo "SKIP: sandbox-exec could not run the probe: $denied"; exit 0; }
    ;;
  *)
    echo "SKIP: no ICMP-denial mechanism known for $(uname -s)"; exit 0 ;;
esac

if [ "$denied" != "raised" ]; then
  echo "FAIL: under an ICMP denial net::ping must RAISE, not return a status" >&2
  echo "      got: '$denied'" >&2
  exit 1
fi
echo "PASS: net::ping raises (never a PingStatus) when the OS refuses an ICMP socket"
