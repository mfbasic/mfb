#!/usr/bin/env node
// diff.mjs — run `packages/mustache` and mustache.js over the same templates
// and compare, and run both against the official Mustache specification suite.
//
//     npm install
//     ./fetch-spec.sh
//     node diff.mjs                     # every mode
//     node diff.mjs spec                # one mode
//     node diff.mjs fuzz --count 2000   # more cases
//
// Exit status is 0 iff every specification case passed and every differential
// case agreed, or diverged for a reason declared in divergences.json.
//
// Four modes, and the first two answer different questions on purpose:
//
//   spec     the official suite, github.com/mustache/spec — the six REQUIRED
//            modules, 136 cases. This is a conformance check, not a comparison:
//            each case carries its own `expected` output. It also reports which
//            cases mustache.js itself fails, which is how the one construct
//            where this package deliberately differs from the reference stays
//            visible instead of looking like a bug.
//   corpus   hand-written whole documents in corpus/, compared against
//            mustache.js. The specification is a set of minimal cases; these
//            are realistic templates, so a failure names a construct in the
//            shape someone would actually write it.
//   fuzz     generated contexts with generated templates that reference them,
//            compared against mustache.js. Reaches combinations — a partial
//            inside an inverted section inside a delimiter change — that no
//            hand-written corpus covers.
//   mutate   corpus templates with random byte-level damage. This one asserts
//            ROBUSTNESS, not agreement: whatever the damage, the renderer must
//            answer with a well-formed envelope rather than crash, hang, or
//            print something unparseable. Agreement is reported as information.

import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { renderCase } from './oracle.mjs'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = join(HERE, '..', '..', '..')
const PROBE = join(HERE, 'probe', 'build', 'mustacheprobe.out')
const CORPUS = join(HERE, 'corpus')
const SPEC = join(HERE, 'spec')
const SCRATCH = join(REPO, 'target', 'mustache-oracle-node')
const SCRATCH_FILE = join(SCRATCH, 'case.json')
const DIVERGENCES = JSON.parse(readFileSync(join(HERE, 'divergences.json'), 'utf8'))

const SPEC_MODULES = ['comments', 'delimiters', 'interpolation', 'inverted', 'partials', 'sections']

// ---------------------------------------------------------------------------
// Running the two renderers
// ---------------------------------------------------------------------------

/**
 * Run the MFBASIC probe on a case file. A non-zero exit or an unparseable line
 * is a HARNESS failure, distinct from the renderer refusing a template — the
 * probe reports a refusal on stdout with exit 0 precisely so the two cannot be
 * confused.
 */
function runProbe(path) {
  let stdout
  try {
    stdout = execFileSync(PROBE, [path], { encoding: 'utf8', timeout: 30_000 })
  } catch (problem) {
    return {
      broken: `probe exited ${problem.status ?? problem.signal ?? problem.message}: ${String(problem.stderr ?? '').trim()}`,
    }
  }
  try {
    return JSON.parse(stdout)
  } catch {
    return { broken: `probe printed something that is not JSON: ${JSON.stringify(stdout.slice(0, 200))}` }
  }
}

function probeCase(source) {
  mkdirSync(SCRATCH, { recursive: true })
  writeFileSync(SCRATCH_FILE, JSON.stringify(source))
  return runProbe(SCRATCH_FILE)
}

/**
 * Compare two envelopes. Returns null on agreement, or a description of how
 * they differ.
 *
 * Both refusing counts as agreement WITHOUT comparing the reasons: the two
 * implementations have their own vocabularies for "no", and demanding the same
 * words would make the harness fail on wording. What matters is that neither
 * one silently rendered a template the other refused.
 *
 * An `oracle-defect` is neither: mustache.js crashed rather than answered, so
 * there is nothing to compare against. It is reported with its own prefix and
 * counted separately — never as a package failure, and never silently, because
 * a growing count of them would mean the oracle had stopped being useful.
 */
function compare(mine, theirs) {
  if (mine.broken) return `harness: ${mine.broken}`
  if (!theirs.ok && theirs.kind === 'oracle-defect') return `oracle-defect: ${theirs.reason}`
  if (!mine.ok && !theirs.ok) return null
  if (mine.ok && !theirs.ok) {
    return `mfb rendered, oracle refused (${theirs.reason})\n    mfb:    ${JSON.stringify(mine.output)}`
  }
  if (!mine.ok && theirs.ok) {
    return `oracle rendered, mfb refused ([${mine.code}] ${mine.message})\n    oracle: ${JSON.stringify(theirs.output)}`
  }
  if (mine.output === theirs.output) return null
  return `both rendered, output differs\n    mfb:    ${JSON.stringify(mine.output)}\n    oracle: ${JSON.stringify(theirs.output)}`
}

function checkCase(source) {
  return compare(probeCase(source), renderCase(source))
}

// ---------------------------------------------------------------------------
// spec
// ---------------------------------------------------------------------------

function modeSpec() {
  if (!existsSync(SPEC)) {
    console.log('FAIL spec/ is missing. Download the official suite first:')
    console.log('     ./fetch-spec.sh')
    return { cases: 0, failures: 1 }
  }
  let failures = 0
  let cases = 0
  let oracleFailures = 0
  for (const module of SPEC_MODULES) {
    const path = join(SPEC, `${module}.json`)
    if (!existsSync(path)) {
      console.log(`FAIL spec/${module}.json is missing — re-run ./fetch-spec.sh`)
      failures += 1
      continue
    }
    for (const test of JSON.parse(readFileSync(path, 'utf8')).tests) {
      cases += 1
      const source = { template: test.template, data: test.data, partials: test.partials ?? {} }
      const mine = probeCase(source)
      const got = mine.broken ? null : mine.ok ? mine.output : `<refused [${mine.code}] ${mine.message}>`
      if (mine.broken) {
        console.log(`FAIL ${module}/${test.name}: harness: ${mine.broken}`)
        failures += 1
      } else if (got !== test.expected) {
        console.log(`FAIL ${module}/${test.name}`)
        console.log(`     ${test.desc}`)
        console.log(`     template ${JSON.stringify(test.template)}`)
        console.log(`     data     ${JSON.stringify(test.data)}`)
        if (test.partials) console.log(`     partials ${JSON.stringify(test.partials)}`)
        console.log(`     want     ${JSON.stringify(test.expected)}`)
        console.log(`     got      ${JSON.stringify(got)}`)
        failures += 1
      }
      // Informational: the reference implementation is not itself conformant.
      const theirs = renderCase(source)
      if (!theirs.ok || theirs.output !== test.expected) oracleFailures += 1
    }
  }
  return {
    cases,
    failures,
    note: `mustache.js itself fails ${oracleFailures} of them (informational)`,
  }
}

// ---------------------------------------------------------------------------
// corpus
// ---------------------------------------------------------------------------

function corpusFiles() {
  const files = []
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true }).sort((x, y) => x.name.localeCompare(y.name))) {
      const full = join(dir, entry.name)
      if (entry.isDirectory()) walk(full)
      else if (entry.name.endsWith('.json')) files.push(full)
    }
  }
  walk(CORPUS)
  return files
}

function modeCorpus() {
  let failures = 0
  let declared = 0
  let defects = 0
  const files = corpusFiles()
  for (const file of files) {
    const name = relative(CORPUS, file)
    const source = JSON.parse(readFileSync(file, 'utf8'))
    const reason = DIVERGENCES[name]
    const difference = checkCase(source)

    if (difference && difference.startsWith('oracle-defect:')) {
      defects += 1
      console.log(`skip ${name}: ${difference}`)
      continue
    }

    if (reason) {
      declared += 1
      if (!difference) {
        console.log(`FAIL ${name}: declared to diverge (${reason}) but AGREED`)
        console.log('     If the package now matches mustache.js here, delete its divergences.json row.')
        failures += 1
      } else if (difference.startsWith('harness:')) {
        console.log(`FAIL ${name}: ${difference}`)
        failures += 1
      }
      continue
    }

    if (difference) {
      console.log(`FAIL ${name}: ${difference}`)
      failures += 1
    }
  }
  const defectNote = defects > 0 ? `, ${defects} the oracle could not judge` : ''
  return { cases: files.length, failures, note: `${declared} declared divergence(s)${defectNote}` }
}

// ---------------------------------------------------------------------------
// fuzz
// ---------------------------------------------------------------------------

// A deterministic generator, so a failure is reproducible from its seed.
function makeRandom(seed) {
  let state = seed >>> 0
  return () => {
    state = (state * 1664525 + 1013904223) >>> 0
    return state / 4294967296
  }
}

const pick = (random, list) => list[Math.floor(random() * list.length)]

const WORDS = [
  'alpha', 'beta gamma', '', 'x', 'a & b', '<em>hi</em>', '"quoted"', "it's",
  'path/to/file', '`tick`', 'k=v', 'café 中文 😀', '  padded  ', 'line\nbreak',
  '{{notatag}}', '}}', '{{', '\t', '0', 'null', 'true',
]

const SCALARS = [
  (r) => pick(r, WORDS),
  (r) => Math.floor(r() * 2e6) - 1e6,
  (r) => Math.round((r() * 2e4 - 1e4) * 1e6) / 1e6,
  (r) => r() < 0.5,
  () => null,
]

/**
 * Build a random context, and collect every path into it with the kind of
 * value it names.
 *
 * Every key in a case is UNIQUE — `n0`, `n1`, ... from one counter — so no name
 * in one context frame can shadow a name in another. That is deliberate: it
 * keeps the generator away from the one construct on which mustache.js and the
 * specification disagree, a dotted name whose first segment resolves in the
 * inner frame while its remainder only resolves in an outer one. That is a
 * stated decision, pinned in corpus/divergent/, and a fuzzer rediscovering it a
 * few hundred times a run would bury everything else. Unique keys also keep the
 * recorded KIND of a path honest: two frames sharing a key but not its type
 * would make the template generator interpolate something it believed was a
 * scalar.
 */
function buildContext(random, depth, prefix, paths, counter) {
  const object = {}
  const count = 1 + Math.floor(random() * 3)
  for (let i = 0; i < count; i += 1) {
    const key = `n${counter.next++}`
    const path = prefix ? `${prefix}.${key}` : key
    const roll = random()
    if (depth >= 2 || roll < 0.55) {
      const value = pick(random, SCALARS)(random)
      object[key] = value
      paths.push({ path, kind: 'scalar' })
    } else if (roll < 0.78) {
      const items = []
      const size = Math.floor(random() * 4)
      const scalarItems = random() < 0.5
      for (let j = 0; j < size; j += 1) {
        items.push(scalarItems ? pick(random, SCALARS)(random) : buildContext(random, depth + 1, '', paths, counter).value)
      }
      object[key] = items
      paths.push({ path, kind: 'list', scalarItems })
    } else {
      const nested = buildContext(random, depth + 1, path, paths, counter)
      object[key] = nested.value
      paths.push({ path, kind: 'object' })
    }
  }
  return { value: object }
}

const DELIMITER_PAIRS = [['<%', '%>'], ['[[', ']]'], ['((', '))'], ['|', '|'], ['<<', '>>']]

/**
 * Build a template that references `paths`. `open`/`close` are the delimiters in
 * force, which a `{{= =}}` tag changes for everything after it.
 */
function buildTemplate(random, paths, partialNames, depth) {
  let out = ''
  let open = '{{'
  let close = '}}'
  // Sections recurse, so both the branching factor and the nesting are bounded
  // here rather than left to the dice.
  const chunks = 1 + Math.floor(random() * (depth < 2 ? 6 : 3))
  for (let i = 0; i < chunks; i += 1) {
    const roll = random()
    const named = paths.length > 0 ? pick(random, paths) : null
    const missing = `absent${Math.floor(random() * 3)}`

    if (roll < 0.2) {
      out += pick(random, WORDS)
      if (random() < 0.4) out += '\n'
      continue
    }
    if (roll < 0.28) {
      // A standalone comment line: whitespace, the tag, and the newline all go.
      out += `${' '.repeat(Math.floor(random() * 3))}${open}! ${pick(random, WORDS).replace(/\n/g, ' ')} ${close}\n`
      continue
    }
    if (roll < 0.35 && depth < 2) {
      const [nextOpen, nextClose] = pick(random, DELIMITER_PAIRS)
      out += `${open}=${nextOpen} ${nextClose}=${close}\n`
      open = nextOpen
      close = nextClose
      continue
    }
    if (roll < 0.42 && partialNames.length > 0) {
      const indent = ' '.repeat(Math.floor(random() * 4))
      out += random() < 0.5 ? `${indent}${open}>${pick(random, partialNames)}${close}\n` : `[${open}>${pick(random, partialNames)}${close}]`
      continue
    }
    if (roll < 0.62 || !named || depth >= 3) {
      // An interpolation. Only ever of a scalar or a missing name: the
      // specification says nothing about interpolating a collection, and the
      // two implementations answer differently on purpose (see
      // corpus/divergent/interpolate-collection.json).
      const scalar = paths.filter((p) => p.kind === 'scalar')
      const name = scalar.length > 0 && random() < 0.8 ? pick(random, scalar).path : missing
      const form = random()
      if (form < 0.6) out += `${open}${name}${close}`
      else if (form < 0.8) out += `${open}{${name}}${close}`
      else out += `${open}&${name}${close}`
      continue
    }

    const inverted = random() < 0.3
    const sigil = inverted ? '^' : '#'
    const name = random() < 0.85 ? named.path : missing
    const standalone = random() < 0.5
    const body =
      named.kind === 'list' && named.scalarItems && !inverted && random() < 0.6
        ? `${open}.${close} `
        : buildTemplate(random, paths, partialNames, depth + 1).text
    if (standalone) out += `${open}${sigil}${name}${close}\n${body}\n${open}/${name}${close}\n`
    else out += `${open}${sigil}${name}${close}${body}${open}/${name}${close}`
  }
  return { text: out }
}

function modeFuzz(count) {
  const random = makeRandom(20260906)
  let failures = 0
  let defects = 0
  for (let index = 0; index < count; index += 1) {
    const paths = []
    const data = buildContext(random, 0, '', paths, { next: 0 }).value
    const partials = {}
    const partialNames = []
    for (let p = 0; p < Math.floor(random() * 3); p += 1) {
      const name = `p${p}`
      partialNames.push(name)
      partials[name] = buildTemplate(random, paths, [], 2).text
    }
    const template = buildTemplate(random, paths, partialNames, 0).text
    const source = { template, data, partials }
    const difference = checkCase(source)
    if (difference && difference.startsWith('oracle-defect:')) {
      defects += 1
      continue
    }
    if (difference) {
      console.log(`FAIL fuzz #${index}: ${difference}`)
      console.log(`--- case ---\n${JSON.stringify(source, null, 1)}\n--- end ---`)
      failures += 1
      if (failures > 10) {
        console.log('...stopping after 10 fuzz failures')
        return { cases: index + 1, failures }
      }
    }
  }
  const defectNote = defects > 0 ? `${defects} case(s) crashed the oracle and could not be judged` : ''
  return { cases: count, failures, note: defectNote }
}

// ---------------------------------------------------------------------------
// mutate
// ---------------------------------------------------------------------------

const DAMAGE = ['{', '}', '#', '^', '/', '>', '&', '!', '=', '.', ' ', '\t', '\n', '"', '\\', '', 'x']

function mutate(template, random) {
  const characters = [...template]
  const edits = 1 + Math.floor(random() * 4)
  for (let i = 0; i < edits && characters.length > 0; i += 1) {
    const at = Math.floor(random() * characters.length)
    const roll = random()
    if (roll < 0.4) characters[at] = pick(random, DAMAGE)
    else if (roll < 0.7) characters.splice(at, 1)
    else characters.splice(at, 0, pick(random, DAMAGE))
  }
  return characters.join('')
}

function modeMutate(count) {
  const random = makeRandom(7)
  // Documents already declared to diverge are excluded: every mutation of one
  // diverges too, which would drown the informational agreement rate without
  // telling anyone anything.
  const sources = corpusFiles()
    .filter((file) => !DIVERGENCES[relative(CORPUS, file)])
    .map((file) => JSON.parse(readFileSync(file, 'utf8')))
  let failures = 0
  let agreed = 0
  for (let index = 0; index < count; index += 1) {
    const base = sources[index % sources.length]
    const source = { ...base, template: mutate(base.template, random) }
    const mine = probeCase(source)
    if (mine.broken) {
      console.log(`FAIL mutate #${index}: ${mine.broken}`)
      console.log(`--- template ---\n${source.template}\n--- end ---`)
      failures += 1
      if (failures > 5) {
        console.log('...stopping after 5 robustness failures')
        return { cases: index + 1, failures }
      }
      continue
    }
    const theirs = renderCase(source)
    if (mine.ok === theirs.ok && (!mine.ok || mine.output === theirs.output)) agreed += 1
  }
  const rate = count === 0 ? 0 : Math.round((agreed / count) * 100)
  return { cases: count, failures, note: `${rate}% also agreed (informational)` }
}

// ---------------------------------------------------------------------------

const MODES = { spec: () => modeSpec(), corpus: () => modeCorpus(), fuzz: (n) => modeFuzz(n), mutate: (n) => modeMutate(n) }

function usage(problem) {
  process.stderr.write(`${problem}\nusage: node diff.mjs [spec|corpus|fuzz|mutate ...] [--count N]\n`)
  return 2
}

function main(argv) {
  const countAt = argv.indexOf('--count')
  let count = 400
  if (countAt >= 0) {
    count = Number(argv[countAt + 1])
    argv.splice(countAt, 2)
    if (!Number.isInteger(count) || count < 1) return usage('--count needs a positive integer')
  }
  const wanted = argv.length > 0 ? argv : Object.keys(MODES)
  const unknown = wanted.filter((name) => !(name in MODES))
  if (unknown.length > 0) return usage(`unknown mode(s) ${unknown.join(', ')}`)

  if (!existsSync(PROBE)) {
    process.stderr.write(
      `${PROBE} is missing. Build it first:\n` +
        '  mfb build packages/mustache\n' +
        '  mkdir -p packages/mustache/oracle/probe/packages\n' +
        '  cp packages/mustache/mustache.mfp packages/mustache/oracle/probe/packages/mustache.mfp\n' +
        '  mfb build packages/mustache/oracle/probe\n',
    )
    return 1
  }

  let total = 0
  for (const name of wanted) {
    const { cases, failures, note } = MODES[name](count)
    total += failures
    console.log(`${name}: ${cases} case(s), ${failures} failure(s)${note ? ` — ${note}` : ''}`)
  }
  if (total === 0) {
    console.log('\nEvery specification case passed, and every comparison agreed or diverged for a declared reason (divergences.json).')
  }
  return total === 0 ? 0 : 1
}

process.exit(main(process.argv.slice(2)))
