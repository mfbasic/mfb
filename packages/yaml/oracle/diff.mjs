#!/usr/bin/env node
// diff.mjs — run `packages/yaml` and the `yaml` npm module over the same input
// and compare.
//
//     npm install
//     node diff.mjs                     # every mode
//     node diff.mjs corpus              # one mode
//     node diff.mjs fuzz --count 2000   # more cases
//
// Exit status is 0 iff every case agreed, or diverged for a reason declared in
// divergences.json.
//
// Three modes, in increasing order of how hard they are to satisfy:
//
//   corpus   the hand-written documents in corpus/. Readable, greppable, and
//            each one is a whole realistic document rather than a snippet, so a
//            failure names a construct instead of a line number.
//   fuzz     random values serialized to YAML BY THE ORACLE'S OWN WRITER, in
//            varied styles. The writer only ever emits YAML its own reader
//            accepts, so any disagreement is ours, and the styles it cycles
//            through (block/flow, indents, line widths, quote preferences)
//            reach shapes no hand-written corpus covers.
//   mutate   corpus documents with random byte-level damage. This one asserts
//            ROBUSTNESS, not agreement: whatever the damage, the reader must
//            answer with a well-formed envelope rather than crash, hang, or
//            print something unparseable. Agreement is reported as information.

import { execFileSync } from 'node:child_process'
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { stringify } from 'yaml'
import { OPTIONS, readYaml } from './oracle.mjs'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = join(HERE, '..', '..', '..')
const PROBE = join(HERE, 'probe', 'build', 'yamlprobe.out')
const CORPUS = join(HERE, 'corpus')
const SCRATCH = join(REPO, 'target', 'yaml-oracle-node')
const DIVERGENCES = JSON.parse(readFileSync(join(HERE, 'divergences.json'), 'utf8'))

const SCRATCH_FILE = join(SCRATCH, 'case.yaml')

// ---------------------------------------------------------------------------
// Running the two readers
// ---------------------------------------------------------------------------

/**
 * Run the MFBASIC probe. A non-zero exit or an unparseable line is a HARNESS
 * failure, distinct from the reader refusing a document — the probe reports a
 * refusal on stdout with exit 0 precisely so the two cannot be confused.
 */
function runProbe(path) {
  let stdout
  try {
    stdout = execFileSync(PROBE, [path], { encoding: 'utf8', timeout: 30_000 })
  } catch (problem) {
    return {
      broken: `probe exited ${problem.status ?? problem.signal ?? '?'}: ${String(problem.stderr ?? '').trim()}`,
    }
  }
  try {
    return JSON.parse(stdout)
  } catch {
    return { broken: `probe printed something that is not JSON: ${JSON.stringify(stdout.slice(0, 200))}` }
  }
}

/** Order-independent, formatting-independent text for one envelope's value. */
function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`
  if (value && typeof value === 'object') {
    const keys = Object.keys(value).sort()
    return `{${keys.map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`).join(',')}}`
  }
  return JSON.stringify(value)
}

/**
 * Compare two envelopes. Returns null on agreement, or a description of how
 * they differ.
 *
 * Both refusing counts as agreement WITHOUT comparing the reasons: the two
 * implementations have their own vocabularies for "no", and demanding the same
 * words would make the harness fail on wording. What matters is that neither
 * one silently produced a value the other refused.
 */
function compare(mine, theirs) {
  if (mine.broken) return `harness: ${mine.broken}`
  if (!mine.ok && !theirs.ok) return null
  if (mine.ok && !theirs.ok) {
    return `mfb accepted, oracle refused (${theirs.kind}: ${theirs.reason})\n    mfb:    ${canonical(mine.documents)}`
  }
  if (!mine.ok && theirs.ok) {
    return `oracle accepted, mfb refused ([${mine.code}] ${mine.message})\n    oracle: ${canonical(theirs.documents)}`
  }
  const a = canonical(mine.documents)
  const b = canonical(theirs.documents)
  if (a === b) return null
  return `both accepted, values differ\n    mfb:    ${a}\n    oracle: ${b}`
}

function checkSource(source) {
  mkdirSync(SCRATCH, { recursive: true })
  writeFileSync(SCRATCH_FILE, source)
  return compare(runProbe(SCRATCH_FILE), readYaml(source))
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
      else if (entry.name.endsWith('.yaml')) files.push(full)
    }
  }
  walk(CORPUS)
  return files
}

function modeCorpus() {
  let failures = 0
  let expectedSeen = 0
  const files = corpusFiles()
  for (const file of files) {
    const name = relative(CORPUS, file)
    const source = readFileSync(file, 'utf8')
    const reason = DIVERGENCES[name]
    // The probe reads the corpus file directly, so a case can also be
    // reproduced by hand with exactly the command the harness ran.
    const difference = compare(runProbe(file), readYaml(source))

    if (reason) {
      expectedSeen += 1
      if (!difference) {
        console.log(`FAIL ${name}: declared to diverge (${reason}) but AGREED`)
        console.log('     If the package now accepts this, delete its divergences.json row.')
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
  return { cases: files.length, failures, note: `${expectedSeen} declared divergence(s)` }
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

const WORDS = [
  'alpha', 'beta', 'key name', 'x', 'a b c', '', 'line one\nline two\n',
  'line one\nline two', 'yes', 'no', 'true', 'null', '12', '1.5', '- dash',
  '#hash', 'a: b', 'tab\there', 'café 中文 😀', '  padded  ', 'ends:', '*star',
  '&amp', '[brack]', '{brace}', "q'single", 'q"double', 'back\\slash',
  '.inf', '.nan', '0x1F', '~', '---', '...', '<<', '? question', '|pipe',
]

function randomValue(random, depth = 0) {
  const roll = random()
  if (depth > 3 || roll < 0.35) {
    const pick = random()
    if (pick < 0.48) return WORDS[Math.floor(random() * WORDS.length)]
    if (pick < 0.66) return Math.floor(random() * 2e9) - 1e9
    if (pick < 0.79) return Math.round((random() * 2e4 - 1e4) * 1e6) / 1e6
    if (pick < 0.91) return random() < 0.5
    return null
  }
  if (roll < 0.68) {
    const items = []
    for (let i = 0; i < Math.floor(random() * 5); i += 1) items.push(randomValue(random, depth + 1))
    return items
  }
  const object = {}
  const suffixes = ['a', 'b c', 'd-e', ':colon', '#h', 'yes', '12']
  for (let i = 0; i < Math.floor(random() * 5); i += 1) {
    object[`k${i} ${suffixes[Math.floor(random() * suffixes.length)]}`] = randomValue(random, depth + 1)
  }
  return object
}

// Writer styles to cycle through. Each reaches shapes the others do not: flow
// style, narrow line widths that force folding, and wide indents.
const STYLES = [
  {},
  { defaultStringType: 'QUOTE_DOUBLE' },
  { defaultStringType: 'QUOTE_SINGLE' },
  { lineWidth: 20 },
  { lineWidth: 0 },
  { indent: 4 },
  { indentSeq: false },
  { flowCollectionPadding: false },
]

function modeFuzz(count) {
  const random = makeRandom(20260905)
  let failures = 0
  for (let index = 0; index < count; index += 1) {
    const value = randomValue(random)
    const style = STYLES[index % STYLES.length]
    let source
    try {
      source = stringify(value, { ...OPTIONS, ...style })
    } catch (problem) {
      // The writer refusing to write a value is not a finding about the reader.
      continue
    }
    const difference = checkSource(source)
    if (difference) {
      console.log(`FAIL fuzz #${index} (style ${JSON.stringify(style)}): ${difference}`)
      console.log(`--- source ---\n${source}--- end ---`)
      failures += 1
      if (failures > 10) {
        console.log('...stopping after 10 fuzz failures')
        return { cases: index + 1, failures }
      }
    }
  }
  return { cases: count, failures }
}

// ---------------------------------------------------------------------------
// mutate
// ---------------------------------------------------------------------------

const DAMAGE = ['\t', ' ', ':', '-', '#', '"', "'", '[', ']', '{', '}', '&', '*', '!', '|', '>', '%', '\\', '\n', '', 'x']

function mutate(source, random) {
  const bytes = [...source]
  const edits = 1 + Math.floor(random() * 3)
  for (let i = 0; i < edits && bytes.length > 0; i += 1) {
    const at = Math.floor(random() * bytes.length)
    const roll = random()
    if (roll < 0.4) bytes[at] = DAMAGE[Math.floor(random() * DAMAGE.length)]
    else if (roll < 0.7) bytes.splice(at, 1)
    else bytes.splice(at, 0, DAMAGE[Math.floor(random() * DAMAGE.length)])
  }
  return bytes.join('')
}

function modeMutate(count) {
  const random = makeRandom(7)
  // Documents already declared to diverge are excluded: every mutation of one
  // diverges too, which would drown the informational agreement rate without
  // telling anyone anything.
  const sources = corpusFiles()
    .filter((file) => !DIVERGENCES[relative(CORPUS, file)])
    .map((file) => readFileSync(file, 'utf8'))
  let failures = 0
  let agreed = 0
  for (let index = 0; index < count; index += 1) {
    const source = mutate(sources[index % sources.length], random)
    mkdirSync(SCRATCH, { recursive: true })
    writeFileSync(SCRATCH_FILE, source)
    const mine = runProbe(SCRATCH_FILE)
    if (mine.broken) {
      console.log(`FAIL mutate #${index}: ${mine.broken}`)
      console.log(`--- source ---\n${source}\n--- end ---`)
      failures += 1
      if (failures > 5) {
        console.log('...stopping after 5 robustness failures')
        return { cases: index + 1, failures }
      }
      continue
    }
    const theirs = readYaml(source)
    if (mine.ok === theirs.ok && (!mine.ok || canonical(mine.documents) === canonical(theirs.documents))) {
      agreed += 1
    }
  }
  const rate = count === 0 ? 0 : Math.round((agreed / count) * 100)
  return { cases: count, failures, note: `${rate}% also agreed (informational)` }
}

// ---------------------------------------------------------------------------

const MODES = { corpus: () => modeCorpus(), fuzz: (n) => modeFuzz(n), mutate: (n) => modeMutate(n) }

function main(argv) {
  const countAt = argv.indexOf('--count')
  let count = 600
  if (countAt >= 0) {
    count = Number(argv[countAt + 1])
    argv.splice(countAt, 2)
    if (!Number.isInteger(count) || count < 1) return usage('--count needs a positive integer')
  }
  const wanted = argv.length > 0 ? argv : Object.keys(MODES)
  const unknown = wanted.filter((name) => !(name in MODES))
  if (unknown.length > 0) return usage(`unknown mode(s) ${unknown.join(', ')}`)

  try {
    execFileSync(PROBE, ['--version'], { stdio: 'ignore' })
  } catch (problem) {
    if (problem.code === 'ENOENT') {
      process.stderr.write(
        `${PROBE} is missing. Build it first:\n` +
          '  mfb build packages/yaml\n' +
          '  mkdir -p packages/yaml/oracle/probe/packages\n' +
          '  cp packages/yaml/yaml.mfp packages/yaml/oracle/probe/packages/yaml.mfp\n' +
          '  mfb build packages/yaml/oracle/probe\n',
      )
      return 1
    }
    // A non-zero exit from a bogus argument is fine — the binary exists.
  }

  let total = 0
  for (const name of wanted) {
    const { cases, failures, note } = MODES[name](count)
    total += failures
    console.log(`${name}: ${cases} case(s), ${failures} failure(s)${note ? ` — ${note}` : ''}`)
  }
  if (total === 0) {
    console.log('\nEvery case agreed, or diverged for a declared reason (divergences.json).')
  }
  return total === 0 ? 0 : 1
}

function usage(problem) {
  process.stderr.write(`${problem}\nusage: node diff.mjs [corpus|fuzz|mutate ...] [--count N]\n`)
  return 2
}

process.exit(main(process.argv.slice(2)))
