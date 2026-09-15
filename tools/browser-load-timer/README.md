# tools/browser-load-timer

Times how long `examples/browser` takes to load one page, from pressing Enter in Address
Mode to the page being drawn. plan-133-B (entropy fill on vs off) and plan-133-C (the cost of
the `--debug` memory series) compare this number between two builds of the same browser.

    bash tools/browser-load-timer/run.sh <browser.out> <url> <runs>

Runs the browser `<runs>` times, back to back, and prints one row per run and then the
median:

    run 1 load_ms=12345 exit=0
    ...
    median load_ms=12345 over 3 runs

- Each run starts the browser fresh in a 120×40 pty (`load.exp`, needs `expect`), presses
  `G`, clears the address field, types `<url>`, and presses Enter.
- `load_ms` stops when the footer's `Files: n/m` count changes from `0/0` (row 40 of the
  pty). The browser sets that count only when the page's load result returns
  (`examples/browser/app/src/main.mfb`, the load section), so the time covers the fetch,
  parse, stylesheets, style resolution and the copy back to the main thread.
- The browser redraws only changed cells, so the driver matches the rewritten digits (a
  cursor move to row 40, optional colour escapes, a non-zero digit), not the `Files:` label
  or the padlock glyph: neither is sent again after the first frame. Set
  `MFB_TIMER_LOG=<file>` to keep every byte the browser drew when a run times out.
- The driver then presses `q` and waits for the process to exit. `exit` is its status; a
  run that never finishes loading prints `load_ms=timeout` after 120 s and is stopped.
- A live page's timings include the network. Compare two builds with runs back to back on
  the same host, and use the median.
- A `--debug` build prints its report to stderr, which `load.exp` leaves on the terminal
  stream it reads. Pass `MFB_TIMER_STDERR=<file>` to keep the report separate (the browser
  is then run under `sh -c '… 2>file'`).
