#!/usr/bin/env bash
# Runs every shell example in the `mfb man debug` and `mfb man debug-external`
# topics on the platform it is for.
#
# The topics' scripts live in src/docs/man/debug/*.md and
# src/docs/man/debug-external/*.md as fenced blocks tagged by platform:
#
#   ```sh macos            run with bash on this Mac, from a scratch copy of the repo
#   ```sh linux            run with bash on a Linux box over ssh
#   ```powershell windows  run with Windows PowerShell on a Windows box over ssh
#   ```powershell windows-setup   the w64devkit install; run only with --setup
#
# (The man renderer ignores a fence's tag, so the tags are invisible to readers.)
#
# Every block starts from a scratch directory holding `examples/debug` and nothing
# else, the layout the topic tells a reader to run from (the repository root).
# On the Mac the block's `mfb build` runs for real. A Linux or Windows box has no
# `mfb`, so the harness runs each `mfb build ...` line of the block HERE, with the
# same flags plus `--target`, ships the scratch tree, and puts a stub `mfb` on the
# remote PATH that only checks the build output is present. Everything after the
# build — the debugger, the profiler, the lookups — runs on the target OS.
#
# A block passes when it exits 0 and prints at least one line naming an MFBASIC
# function (`mfb_fn_`), a debug report's end (`mfb.debug.end`), its main arena
# (`arena.0.kind`) or the end of an `mfb dbg` session (`DBG> q`); the setup block
# passes when gdb prints its version. Each block's output is kept in
# $OUT/<platform>-<page>-<n>.log for refreshing the topics' sample output.
#
# The macOS blocks run `mfb dbg`, which macOS lets pause a program only when it is
# signed for it, so the harness signs <mfb-exe> with scripts/sign-debugger.sh first.
#
# Usage: man-debug-examples.sh <mfb-exe> [macos|linux|windows|all] [--setup]
#
# Environment (defaults are the boxes in .ai/remote_systems.md):
#   LINUX_SSH       ssh destination args for the Linux box  ("-p 2226 test@127.0.0.1")
#   LINUX_TARGET    target to cross-build for               (linux-aarch64)
#   LINUX_TOOLS     optional rootless tool prefix: <prefix>/usr/bin joins PATH and
#                   <prefix>/usr/lib/<triple> joins LD_LIBRARY_PATH (/tmp/p166tools/root;
#                   empty to use the box's own gdb/binutils)
#   WINDOWS_SSH     ssh destination for the Windows box     (test@192.168.1.206)
#   WINDOWS_TOOLS   w64devkit bin directory on that box     (C:\Users\test\w64devkit\bin)
#   OUT             log directory                           (/tmp/man-debug-examples)
set -u

mfb=${1:?usage: man-debug-examples.sh <mfb-exe> [macos|linux|windows|all] [--setup]}
which_os=${2:-all}
setup=0
[ "${3:-}" = "--setup" ] && setup=1
mfb=$(cd "$(dirname "$mfb")" && pwd)/$(basename "$mfb")
repo=$(cd "$(dirname "$0")/.." && pwd)
docs=("$repo/src/docs/man/debug" "$repo/src/docs/man/debug-external")
OUT=${OUT:-/tmp/man-debug-examples}
LINUX_SSH=${LINUX_SSH:-"-p 2226 test@127.0.0.1"}
LINUX_TARGET=${LINUX_TARGET:-linux-aarch64}
LINUX_TOOLS=${LINUX_TOOLS-/tmp/p166tools/root}
WINDOWS_SSH=${WINDOWS_SSH:-test@192.168.1.206}
WINDOWS_TOOLS=${WINDOWS_TOOLS:-'C:\Users\test\w64devkit\bin'}
rm -rf "$OUT"
mkdir -p "$OUT/blocks"
fail=0
pass=0

# Split every page into one file per tagged block: $OUT/blocks/<tag>.<page>.<n>,
# <page> prefixed with `external-` for a debug-external page.
for page in "${docs[0]}"/*.md "${docs[1]}"/*.md; do
	name=$(basename "$page" .md)
	name=${name#[0-9][0-9]_}
	case $page in */debug-external/*) name=external-$name ;; esac
	awk -v dir="$OUT/blocks" -v page="$name" '
		/^```(sh macos|sh linux|powershell windows|powershell windows-setup)$/ {
			tag = $2; n[tag]++; file = dir "/" tag "." page "." n[tag]; next
		}
		/^```/ && file != "" { close(file); file = ""; next }
		file != "" { print > file }
	' "$page"
done

scratch() {
	local dir=$1
	rm -rf "$dir"
	mkdir -p "$dir/examples"
	cp -R "$repo/examples/debug" "$dir/examples/debug"
	rm -rf "$dir/examples/debug/build"
}

check() {
	local label=$1 status=$2 log=$3
	local expect=${4:-'mfb_fn_|mfb\.debug\.end|arena\.0\.kind|^DBG> q$'}
	if [ "$status" = 0 ] && grep -qE "$expect" "$log"; then
		echo "PASS  $label"
		pass=$((pass + 1))
	else
		echo "FAIL  $label (exit $status; see $log)"
		fail=$((fail + 1))
	fi
}

# Run each `mfb build ...` line of a block here, for a target. Windows paths in the
# line (`examples\debug`) are turned into `/` first.
prebuild() {
	local block=$1 dir=$2 target=$3
	grep -E '^mfb build ' "$block" | tr '\\' '/' | while read -r _ _ args; do
		(cd "$dir" && "$mfb" build -q --target "$target" $args) >/dev/null || return 1
	done
}

run_macos() {
	sh "$repo/scripts/sign-debugger.sh" "$mfb" >/dev/null 2>&1 || {
		echo "FAIL  signing $mfb for mfb dbg"
		fail=$((fail + 1))
		return
	}
	for block in "$OUT"/blocks/macos.*; do
		[ -e "$block" ] || continue
		local label=macos-${block#*/blocks/macos.}
		local dir=$OUT/scratch-$label log=$OUT/$label.log
		scratch "$dir"
		(cd "$dir" && PATH=$(dirname "$mfb"):$PATH bash -eo pipefail "$block") >"$log" 2>&1
		check "$label" $? "$log"
	done
}

run_linux() {
	# shellcheck disable=SC2086
	ssh $LINUX_SSH 'rm -rf /tmp/man-debug-examples && mkdir -p /tmp/man-debug-examples'
	for block in "$OUT"/blocks/linux.*; do
		[ -e "$block" ] || continue
		local label=linux-${block#*/blocks/linux.}
		local dir=$OUT/scratch-$label log=$OUT/$label.log
		scratch "$dir"
		if ! prebuild "$block" "$dir" "$LINUX_TARGET"; then
			echo "build failed" >"$log"
			check "$label" 1 "$log"
			continue
		fi
		mkdir -p "$dir/stub"
		printf '#!/bin/sh\n[ "$1" = build ] || exit 2\nls examples/debug/build/debug-*.out >/dev/null\n' >"$dir/stub/mfb"
		chmod +x "$dir/stub/mfb"
		cp "$block" "$dir/block.sh"
		local remote=/tmp/man-debug-examples/$label
		local env="PATH=$remote/stub:\$PATH"
		if [ -n "$LINUX_TOOLS" ]; then
			env="PATH=$remote/stub:$LINUX_TOOLS/usr/bin:\$PATH LD_LIBRARY_PATH=\$(ls -d $LINUX_TOOLS/usr/lib/*-linux-gnu | tr '\n' :)"
		fi
		# shellcheck disable=SC2086
		COPYFILE_DISABLE=1 tar --no-xattrs -C "$dir" -cf - examples stub block.sh | ssh $LINUX_SSH "mkdir -p $remote && tar -C $remote -xf -"
		# shellcheck disable=SC2086
		ssh $LINUX_SSH "cd $remote && export $env && bash -eo pipefail block.sh" >"$log" 2>&1
		check "$label" $? "$log"
	done
}

run_windows() {
	local remote='C:\Users\test\man-debug-examples'
	ssh "$WINDOWS_SSH" "if exist $remote rmdir /s /q $remote"
	ssh "$WINDOWS_SSH" "mkdir $remote"
	local blocks=("$OUT"/blocks/windows.*)
	[ "$setup" = 1 ] && blocks=("$OUT"/blocks/windows-setup.* "${blocks[@]}")
	for block in "${blocks[@]}"; do
		[ -e "$block" ] || continue
		local label=${block#*/blocks/}
		label=${label/windows-setup./windows-setup-}
		label=${label/windows./windows-}
		local dir=$OUT/scratch-$label log=$OUT/$label.log
		scratch "$dir"
		if ! prebuild "$block" "$dir" windows-x86_64; then
			echo "build failed" >"$log"
			check "$label" 1 "$log"
			continue
		fi
		mkdir -p "$dir/stub"
		printf '@echo off\r\nif not "%%1"=="build" exit /b 2\r\nif not exist examples\\debug\\build\\debug.exe exit /b 1\r\n' >"$dir/stub/mfb.cmd"
		# No `$ErrorActionPreference = "Stop"`: Windows PowerShell 5.1 turns a
		# native command's redirected stderr (`2>$null`) into a terminating error
		# under it, which a reader running the same block would never see.
		{
			printf '$env:PATH = "%s\\%s\\stub;%s;" + $env:PATH\r\n' "$remote" "$label" "$WINDOWS_TOOLS"
			printf 'Set-Location "%s\\%s"\r\n' "$remote" "$label"
			sed 's/$/\r/' "$block"
		} >"$dir/block.ps1"
		COPYFILE_DISABLE=1 tar --no-xattrs -C "$dir" -cf "$OUT/$label.tar" examples stub block.ps1
		scp -q "$OUT/$label.tar" "$WINDOWS_SSH:C:/Users/test/man-debug-examples/$label.tar"
		ssh "$WINDOWS_SSH" "mkdir $remote\\$label && tar -C $remote\\$label -xf $remote\\$label.tar"
		ssh "$WINDOWS_SSH" "powershell -NoProfile -ExecutionPolicy Bypass -File $remote\\$label\\block.ps1" >"$log" 2>&1
		local status=$?
		if [[ $label == windows-setup-* ]]; then
			check "$label" $status "$log" 'GNU gdb'
		else
			check "$label" $status "$log"
		fi
	done
}

case $which_os in
macos) run_macos ;;
linux) run_linux ;;
windows) run_windows ;;
all) run_macos; run_linux; run_windows ;;
*) echo "unknown platform '$which_os' (macos|linux|windows|all)" >&2; exit 2 ;;
esac

echo "man-debug-examples [$which_os]: $pass passed, $fail failed (logs in $OUT)"
[ "$fail" = 0 ]
