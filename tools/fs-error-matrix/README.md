# tools/fs-error-matrix

The cross-host `fs` error probe (bug-694, bug-701). One `FUNC … AS Nothing` per case, each
`TRAP`ping a single `fs` call against a fixed set of fixtures and printing
`e.code`, so the same program's output can be diffed across macOS, Linux and
Windows. No gate runs it: the rows that must agree are pinned by
`tests/runtime/rt_fs_host_error_classification.rs` on every host `cargo test`
runs on; this is the full matrix, and the way to measure the Windows column.

- **gen.py** — writes the probe source: `python3 gen.py <dir>/src/main.mfb`.
- **fixtures.sh** / **fixtures.ps1** — build `fx/` in the current directory
  (POSIX / Windows). `fixtures.ps1` then runs `.\probe.exe`. Windows cannot make
  the symbolic-link fixtures without Developer Mode or elevation and prints
  `fixture: no symlink privilege` instead; its ACL fixtures (`ro`, `noread.txt`)
  do not refuse everything the POSIX modes do, and it has no `nox` fixture, so
  those rows differ by fixture, not by mapping.
- **project.json** — the probe project.

Run (from a scratch directory):

    cp tools/fs-error-matrix/project.json /tmp/m/ && mkdir -p /tmp/m/src
    python3 tools/fs-error-matrix/gen.py /tmp/m/src/main.mfb
    cd /tmp/m && sh <repo>/tools/fs-error-matrix/fixtures.sh
    <repo>/target/debug/mfb build . && ./build/probe.out > mac.txt

Linux: `mfb build -target linux-aarch64 .`, copy `build/probe-glibc.out` and
`fixtures.sh` to the box, run `sh fixtures.sh && ./probe-glibc.out`. Windows:
`mfb build -target windows-x86_64 .`, copy `build/probe.exe` and `fixtures.ps1`
to box 2230 (`.ai/remote_systems.md`), run
`powershell -ExecutionPolicy Bypass -File fixtures.ps1`. Diff the outputs.
