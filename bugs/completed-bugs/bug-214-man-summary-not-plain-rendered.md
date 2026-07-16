# bug-214: `mfb man` one-line summaries leak raw Markdown (not render::plain'd like `mfb spec`)

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: correctness

Status: Fixed (2026-07-15) — the `mfb man` package/constant/function one-line summary print sites now run the summary through render::plain (stripping backticks/bold/citations), matching the `mfb spec` path; markdown_summary's doc comment corrected to say it returns raw Markdown that the display sites strip. Verified: `mfb man datetime between` prints "The signed Duration span between two instants." without literal backticks.

Man-page one-line summaries are extracted as raw Markdown and never stripped of
inline markup, so backticks/bold/citations leak verbatim into `mfb man` listings
— unlike the spec path, which wraps `summary_line` in `render::plain(...)`
(`src/docs/spec/mod.rs:63`). The `markdown_summary` doc comment even claims it
mirrors `mfb spec`, which it does not.

Trigger: `mfb man datetime` (or `strings`, `datetime between`, …) prints
`function.summary` raw at `src/cli/man.rs:171`/`226`; e.g. `between.md`'s first
prose line renders as `The signed \`Duration\` span between two instants.` with
literal backticks.

Root cause: `src/docs/man/mod.rs:193` (`markdown_summary`, consumed at 113/168)
returns raw source; doc comment at `:192` misstates the behavior.

Fix: run man summaries through `render::plain` before display (store as `String`,
or strip at the `println!` sites), matching the spec path; correct the comment.
