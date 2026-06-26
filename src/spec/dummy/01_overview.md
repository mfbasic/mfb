# Prose and Lists

Demonstrates paragraphs, inline styling, lists, block quotes, code fences, and
horizontal rules — everything except tables, which live in their own topic.

## Paragraphs

Paragraphs are soft-wrapped: the source can break lines wherever it likes and
the renderer reflows the text to the current terminal width. This sentence is
intentionally long so that it spans several visual lines on a narrow terminal,
letting you confirm that wrapping happens on word boundaries and never splits a
word down the middle.

Inline markup is supported in a small, predictable set: **bold** for emphasis,
*italic* for terms, `inline code` for identifiers like `strings::upper`, and
[labelled links](https://mfbasic.example/docs) that show their URL in plain
output.

## Lists

Unordered lists use a bullet marker:

- A short item.
- A deliberately long item whose text is more than wide enough to wrap onto a
  second visual line, so you can verify that the continuation hangs underneath
  the text rather than under the marker.
- A final item.

Ordered lists keep their numbers:

1. First step.
2. Second step.
3. Third step.

## Code

Fenced code blocks are printed verbatim and never wrapped, even when a line is
wider than the terminal:

```
FUNC greet(name AS String) AS String
  RETURN strings::concat("Hello, ", name)
END FUNC
```

---

That horizontal rule above spans the full terminal width.
