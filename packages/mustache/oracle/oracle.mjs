#!/usr/bin/env node
// oracle.mjs — render one Mustache case with mustache.js and report the result
// as one line of JSON, in exactly the shape `probe/` reports it.
//
//     node oracle.mjs <case.json>
//
//     {"ok":true,"output":"..."}
//     {"ok":false,"kind":"render","reason":"..."}
//
// Why THIS module: mustache.js IS the reference implementation. The Mustache
// specification is a repository of test cases rather than a prose standard, and
// mustache.js is the implementation those cases were written alongside — where
// the specification is silent, what mustache.js does is what template authors
// have learned to expect. Comparing against it therefore covers two different
// things at once: the specification proper (which `diff.mjs`'s `spec` mode
// checks directly, against the official suite) and the large undocumented
// remainder — what a stray `{{`, an unmatched delimiter, a partial with no
// trailing newline, or a name that is only whitespace does.
//
// It is not an oracle for the specification itself. mustache.js fails exactly
// one of the 136 required-module cases ("Dotted Names - Context Precedence"),
// so on that construct the package deliberately differs from it and agrees with
// the specification instead. `divergences.json` declares that, and the `spec`
// mode is what keeps the package honest about which of the two it follows.
//
// A case file is the shape the official suite already uses, so a spec test and
// a hand-written corpus case are the same format:
//
//   {"template": "...", "data": {...}, "partials": {"name": "..."}}

import { readFileSync } from 'node:fs'
import Mustache from 'mustache'

/**
 * Render one case. Returns the same envelope shape the MFBASIC probe prints, so
 * the runner compares two objects rather than two formats.
 *
 * mustache.js throws on a malformed template (an unclosed tag, an unopened or
 * mismatched section). That is a RESULT, not a harness failure, and it is
 * reported as one — the package refuses the same templates, and two refusals
 * count as agreement.
 *
 * A `TypeError` is NOT such a result. mustache.js 4.2.0 crashes on
 * `{{#a}}{{x}}{{/a}}` over `{"a":[null]}`: a section over a list pushes each
 * element as a context frame, and the plain-name branch of `Context.lookup`
 * indexes that frame without a null guard, so a null element throws
 * `Cannot read properties of null`. (The dotted-name branch does guard it,
 * which is why `{{x.y}}` on the same data is fine.) That is a defect in the
 * oracle rather than an answer from it, so it is labelled separately and the
 * runner reports it as a case the oracle could not judge, never as a
 * disagreement. The fuzz mode found it; `corpus/null-list-element.json` pins
 * what the package does with the same input, and the package's own tests
 * assert it independently of this project.
 */
export function renderCase({ template, data, partials }) {
  // The parse cache is keyed on the template text, and `mutate` mode feeds
  // thousands of one-off templates through this process. Clearing it keeps a
  // long run from growing a cache entry per case.
  Mustache.clearCache()
  try {
    return { ok: true, output: Mustache.render(template ?? '', data ?? null, partials ?? {}) }
  } catch (problem) {
    const kind = problem instanceof TypeError ? 'oracle-defect' : 'render'
    return { ok: false, kind, reason: String(problem.message ?? problem) }
  }
}

function main(argv) {
  if (argv.length !== 1) {
    process.stderr.write('usage: node oracle.mjs <case.json>\n')
    return 2
  }
  const source = JSON.parse(readFileSync(argv[0], 'utf8'))
  process.stdout.write(JSON.stringify(renderCase(source)) + '\n')
  return 0
}

if (import.meta.url === `file://${process.argv[1]}`) {
  process.exit(main(process.argv.slice(2)))
}
