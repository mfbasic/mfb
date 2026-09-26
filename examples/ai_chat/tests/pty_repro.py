#!/usr/bin/env python3
"""bug-709 regression test: ai_chat's child-process failure handling.

Drives the built `ai_chat.out` through a pty with throwaway stub `claude`
scripts on PATH (never the real CLI — none of these tests touch the network
or invoke Anthropic's actual CLI). Three independent sub-tests, one per
bug-709 sub-issue:

  (a) discarded exit code + unread stderr — a failed turn must surface its
      exit code and/or stderr text in the transcript, not just "(no
      response)".
  (b) no timeout on waitFor after SIGTERM — closing a session whose child
      ignores SIGTERM must return control to the UI (the app keeps drawing
      frames) within a bounded time, not hang forever.
  (c) prompt goes straight into argv — a prompt that looks like a flag must
      be preceded by a literal "--" in the argv the child receives.

Usage:
    python3 examples/ai_chat/tests/pty_repro.py <path-to-ai_chat.out>

Exits 0 and prints "PASS: <name>" for each sub-test on success. Prints
"FAIL: <name>: <reason>" and exits 1 on the first failure (bounded runtime:
every sub-test has its own hard wall-clock timeout, so a regression that
reintroduces a hang cannot hang this script).
"""
import fcntl
import os
import pty
import select
import signal
import stat
import struct
import sys
import tempfile
import termios
import time

STDLIB_ONLY = True  # uses only pty/select/os/tempfile — no pexpect needed.

# A pty.fork() pty starts at winsize 0x0 (confirmed via TIOCGWINSZ), and
# ai_chat refuses to draw its normal UI below its minCols()/minRows() (it
# shows a "resize your terminal" hint instead, so no keys reach handleInput
# and no child ever spawns). Every pty used below must have its size set
# explicitly before the app's first term::terminalSize() read.
PTY_ROWS, PTY_COLS = 40, 100


def set_pty_size(fd, rows=PTY_ROWS, cols=PTY_COLS):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def make_stub(dirpath, script):
    path = os.path.join(dirpath, "claude")
    with open(path, "w") as f:
        f.write(script)
    st = os.stat(path)
    os.chmod(path, st.st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
    return path


def run_pty(argv, env, keys, gap=1.0, timeout=10.0):
    """Fork a pty, exec argv under env, send `keys` (byte strings) at `gap`
    second intervals, and capture every byte written to the pty master until
    the child exits or `timeout` elapses (force-SIGKILLed on timeout, so this
    can never hang the calling process). Returns (output_bytes, exited,
    exit_status_or_None)."""
    pid, fd = pty.fork()
    if pid == 0:
        try:
            os.execvpe(argv[0], argv, env)
        except Exception:
            os._exit(127)
    set_pty_size(fd)
    output = b""
    start = time.time()
    key_i = 0
    next_key_at = start + gap
    exited = False
    status = None
    while time.time() - start < timeout:
        r, _, _ = select.select([fd], [], [], 0.2)
        if fd in r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            output += chunk
        if key_i < len(keys) and time.time() >= next_key_at:
            try:
                os.write(fd, keys[key_i].encode())
            except OSError:
                pass
            key_i += 1
            next_key_at = time.time() + gap
        try:
            wpid, wstatus = os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            wpid, wstatus = pid, 0
        if wpid == pid:
            exited = True
            status = wstatus
            break
    if not exited:
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass
        try:
            os.waitpid(pid, 0)
        except OSError:
            pass
    try:
        os.close(fd)
    except OSError:
        pass
    return output, exited, status


def env_with_stub(stub_dir):
    env = dict(os.environ)
    env["PATH"] = stub_dir + ":" + env.get("PATH", "")
    env["TERM"] = "xterm-256color"
    return env


def test_a(binary):
    """Discarded exit code + unread stderr: a failing turn must surface its
    exit code / stderr in the transcript."""
    with tempfile.TemporaryDirectory() as d:
        make_stub(d, """#!/bin/bash
echo "fake-claude: fatal: simulated auth failure" >&2
exit 7
""")
        out, _, _ = run_pty(
            [binary], env_with_stub(d),
            keys=["hello", "\r", "\u0003"],
            gap=1.2, timeout=12.0,
        )
        text = out.decode("utf-8", "replace")
        if "simulated auth failure" not in text and "exited 7" not in text and "7" not in text:
            return False, "neither the stub's stderr text nor its exit code (7) appear in the transcript"
        if "simulated auth failure" not in text:
            return False, "exit code may be visible but the stub's stderr text never appears in the transcript"
        return True, None


def test_b(binary):
    """No timeout on waitFor after SIGTERM: closing the app's only session
    (which sets running := FALSE, per handleModal's `len(ns) = 0` case) whose
    child ignores SIGTERM must return control to the UI within a bounded
    time — the process must exit and print "ai_chat: bye." — instead of
    hanging in an unconditional process::waitFor forever. This is exactly the
    bug's own reproduction (bugs/completed/bug-709...: 20s window, force-
    SIGKILLed, "bye." never printed) with a much smaller, still-generous
    bound: the fix's isRunning-poll window is ~2s before it escalates to
    Signal.Kill, so 10s leaves ample slack for process/scheduling overhead
    while still catching a regression back to an unbounded wait."""
    with tempfile.TemporaryDirectory() as d:
        make_stub(d, """#!/bin/bash
trap '' TERM
echo '{"type":"system","subtype":"init","session_id":"deadbeef"}'
while true; do sleep 3600; done
""")
        pid, fd = pty.fork()
        if pid == 0:
            try:
                os.execvpe(binary, [binary], env_with_stub(d))
            except Exception:
                os._exit(127)
        set_pty_size(fd)

        def read_for(seconds):
            deadline = time.time() + seconds
            buf = b""
            while time.time() < deadline:
                r, _, _ = select.select([fd], [], [], 0.2)
                if fd in r:
                    try:
                        chunk = os.read(fd, 65536)
                    except OSError:
                        return buf
                    if not chunk:
                        return buf
                    buf += chunk
            return buf

        output = b""
        try:
            output += read_for(1.5)
            os.write(fd, b"hello")
            output += read_for(1.5)
            os.write(fd, b"\r")
            output += read_for(1.5)
            os.write(fd, b"\x18")  # ^X — open close-session confirm modal
            output += read_for(1.5)
            os.write(fd, b"y")  # confirm close — the app's only session, so
                                 # this sets running := FALSE too

            # Bounded wait for the process to exit (SIGTERM ignored by the
            # stub, so this exercises the isRunning-poll/Signal.Kill path).
            deadline = time.time() + 10.0
            exited = False
            status = None
            while time.time() < deadline:
                r, _, _ = select.select([fd], [], [], 0.2)
                if fd in r:
                    try:
                        chunk = os.read(fd, 65536)
                    except OSError:
                        chunk = b""
                    if chunk:
                        output += chunk
                try:
                    wpid, status = os.waitpid(pid, os.WNOHANG)
                except ChildProcessError:
                    exited = True
                    break
                if wpid == pid:
                    exited = True
                    break
            if not exited:
                return False, "process did not exit within 10s of confirming close — looks hung (bug-709b)"
            if b"ai_chat: bye." not in output:
                return False, "process exited but never printed \"ai_chat: bye.\" — not a clean shutdown"
            return True, None
        finally:
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                pass
            try:
                os.waitpid(pid, 0)
            except (OSError, ChildProcessError):
                pass
            try:
                os.close(fd)
            except OSError:
                pass


def test_c(binary):
    """Prompt goes straight into argv: a prompt starting with "-" must be
    preceded by a literal "--" element in the argv the child receives."""
    with tempfile.TemporaryDirectory() as d:
        argv_log = os.path.join(d, "argv.log")
        make_stub(d, f"""#!/bin/bash
printf '%s\\n' "$@" > {argv_log}
echo '{{"type":"result","subtype":"success","result":"ok"}}'
exit 0
""")
        run_pty(
            [binary], env_with_stub(d),
            keys=["--dangerously-skip-permissions", "\r", "\u0003"],
            gap=1.2, timeout=10.0,
        )
        if not os.path.exists(argv_log):
            return False, "stub claude never ran (argv log missing)"
        with open(argv_log) as f:
            argv = [line.rstrip("\n") for line in f]
        if "--dangerously-skip-permissions" not in argv:
            return False, f"prompt never reached the child's argv: {argv}"
        prompt_idx = argv.index("--dangerously-skip-permissions")
        if prompt_idx == 0 or argv[prompt_idx - 1] != "--":
            return False, f"no literal \"--\" immediately before the prompt in argv: {argv}"
        return True, None


def main():
    if len(sys.argv) != 2:
        print("usage: pty_repro.py <path-to-ai_chat.out>", file=sys.stderr)
        return 2
    binary = os.path.abspath(sys.argv[1])
    if not os.path.isfile(binary) or not os.access(binary, os.X_OK):
        print(f"pty_repro.py: not an executable file: {binary}", file=sys.stderr)
        return 2

    tests = [("a-exit-code-and-stderr", test_a), ("b-sigterm-timeout", test_b), ("c-argv-separator", test_c)]
    failed = False
    for name, fn in tests:
        try:
            ok, reason = fn(binary)
        except Exception as e:
            ok, reason = False, f"exception: {e!r}"
        if ok:
            print(f"PASS: {name}")
        else:
            print(f"FAIL: {name}: {reason}")
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
