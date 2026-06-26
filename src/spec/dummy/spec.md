# Dummy Specification

A throwaway spec package that exercises the `mfb spec` Markdown renderer. It is
the analog of a man `package.txt`: `mfb spec dummy` prints this overview and
lists the subtopics beside it.

This file deliberately uses every supported construct so you can eyeball the
terminal rendering at different widths, e.g. `mfb spec dummy --width 100` versus
`mfb spec dummy --width 50`.

## What this proves

- Headings render as **bold** underlined banners.
- Paragraphs wrap to the terminal width instead of running off the edge.
- Inline markup works: **bold**, *italic*, `code`, and [links](https://example.com).
- Lists keep their markers and hang-indent wrapped lines.

## Subtopics

The pages beside this overview each focus on one construct. See `overview` for
the prose-and-list path and `tables` for the width-aware table reflow, which is
the whole reason specs render through this engine instead of `println!`.

> Note: this package exists only to test the renderer and should be removed once
> a real spec topic is migrated into `src/spec`.
