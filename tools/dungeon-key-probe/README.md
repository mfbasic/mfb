# tools/dungeon-key-probe

Regression probe for bug-713: `examples/dungeon`'s key-reading helper
(`nextKey`, over `io::readKey` since bug-699) must distinguish a malformed
(non-UTF-8) input byte from genuine end of input, instead of conflating both
into `""`/`"EOF"`.

`examples/dungeon` is a `--app` canvas program, so it needs a window and
cannot be driven headlessly by piped stdin. Its key-reading function lives in
its own source file, `examples/dungeon/src/input.mfb` (part of the real
`examples/dungeon` project — it still builds into `dungeon.app` exactly as
before; the split only isolates this function, which touches no canvas API,
from the rest of the game), so `probe.py` builds a small separate console-mode
driver project and copies that file into it **byte-for-byte**
(`shutil.copy`, verified with `filecmp.cmp` before every build) rather than
maintaining a hand-typed or extracted duplicate that could silently drift out
of sync.

(`mfb`'s project loader requires every `sources` root to resolve inside the
project's own directory — `path_within_project` in `src/ast/manifest.rs` — so
a separate driver project cannot point a `sources` root directly at a sibling
example's directory; a byte-identical copy taken immediately before each
build is the closest equivalent available.)

## Usage

```
python3 tools/dungeon-key-probe/probe.py
```

Builds the driver via `target/release/mfb build <tmpdir>` (the caller must
have already built the compiler, `cargo build --release --bin mfb`), runs it
three times with different stdin content, and asserts:

1. A leading `0xFF` byte followed by good bytes is dropped, not read as EOF.
2. Empty stdin still reads as EOF.
3. A clean ASCII byte reads back unchanged.

Exits 0 with PASS lines on success; exits 1 with a diagnostic on the first
failing case (including if the copy of `input.mfb` is ever not byte-identical
to the live source, which would mean the probe was no longer testing the real
code).
