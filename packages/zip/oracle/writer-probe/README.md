# zip writer probe

An MFBASIC executable that writes archives with `packages/zip`, for
`diff.py roundtrip` to read back with Python `zipfile`. It is the only source of
archives in that mode.

```
cp packages/zip/zip.mfp packages/zip/oracle/writer-probe/packages/zip.mfp
mfb build packages/zip/oracle/writer-probe
packages/zip/oracle/writer-probe/build/zipwriter.out <output-directory>
```

It writes five archives into the directory it is given:

- `builder-mixed.zip`, through the in-memory `Builder`, and `stream-mixed.zip`,
  through the streaming `Writer`, with the same calls, so they must be
  byte-identical
- `builder-empty.zip` and `stream-empty.zip`, with no entries
- `entry-count-65536.zip`, 65536 entries, which needs ZIP64

The names, contents, modification times, modes and comment in `src/main.mfb`
must match the expectations in `diff.py` (`mixed_entries`, `entry_count_entries`,
`MIXED_COMMENT`, `T_EVEN`, `T_ODD`). Change both together.

`build/` and `packages/` are gitignored.
