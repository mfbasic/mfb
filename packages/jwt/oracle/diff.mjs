#!/usr/bin/env node
// diff.mjs — ask `packages/jwt` and `jose` the same questions and compare.
//
//     npm install
//     node diff.mjs                       # every mode
//     node diff.mjs corpus                # one mode
//     node diff.mjs fuzz --count 2000
//
// Exit status is 0 iff every case agreed, or diverged for a reason declared in
// divergences.json.
//
// Four modes, in increasing order of how hard they are to satisfy:
//
//   corpus   the hand-written cases in corpus.mjs. Readable and greppable, and
//            each one is a whole rule rather than a snippet, so a failure names
//            a rule instead of a line number.
//   cross    each side signs and the OTHER side verifies, on all eight
//            key/curve combinations. This is the mode that would catch a
//            signature format we got to ourselves — a DER-vs-raw ECDSA
//            signature, an Ed448 half swapped — because neither side ever reads
//            back only its own bytes. For the deterministic algorithms it also
//            compares the two tokens BYTE FOR BYTE.
//   fuzz     random claims, headers and options over every key, signed by one
//            side and verified by both. Seeded, so a failure reproduces.
//   mutate   valid tokens with random byte-level damage. Asserts ROBUSTNESS —
//            whatever the damage, the probe answers with a well-formed envelope
//            rather than crashing, hanging, or printing something unparseable —
//            and that the two still agree on accept/reject.

import { execFileSync } from 'node:child_process'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { base64url } from 'jose'
import { runJob } from './oracle.mjs'
import { KEYS, NOW, SIGNING_KEYS, cases, craft, publicOf, token } from './corpus.mjs'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = join(HERE, '..', '..', '..')
const PROBE = join(HERE, 'probe', 'build', 'jwtprobe.out')
const SCRATCH = join(REPO, 'target', 'jwt-oracle')
const JOB_FILE = join(SCRATCH, 'job.json')
const DIVERGENCES = JSON.parse(readFileSync(join(HERE, 'divergences.json'), 'utf8'))

const ALGORITHMS = [...new Set(SIGNING_KEYS.map((n) => (n.startsWith('EdDSA') ? 'EdDSA' : n)))]

// ---------------------------------------------------------------------------
// Running the two sides
// ---------------------------------------------------------------------------

/**
 * Run the MFBASIC probe over a whole job. A non-zero exit or an unparseable
 * document is a HARNESS failure, distinct from the package refusing a token —
 * the probe reports a refusal inside the document with exit 0 precisely so the
 * two cannot be confused.
 */
function runProbe(job) {
  mkdirSync(SCRATCH, { recursive: true })
  writeFileSync(JOB_FILE, JSON.stringify(job))
  let stdout
  try {
    stdout = execFileSync(PROBE, [JOB_FILE], { encoding: 'utf8', timeout: 300_000, maxBuffer: 64 << 20 })
  } catch (problem) {
    return { broken: `probe exited ${problem.status ?? problem.signal ?? '?'}: ${String(problem.stderr ?? '').trim()}` }
  }
  try {
    return JSON.parse(stdout)
  } catch {
    return { broken: `probe printed something that is not JSON: ${JSON.stringify(stdout.slice(0, 300))}` }
  }
}

/** Order-independent, formatting-independent text for one value. */
function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`
  if (value && typeof value === 'object') {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`)
      .join(',')}}`
  }
  return JSON.stringify(value)
}

/**
 * Compare one case's two answers. Returns null on agreement, or how they differ.
 *
 * Two refusals count as agreement WITHOUT comparing the reasons: the two
 * implementations have their own vocabularies for "no", and demanding the same
 * words would make the harness fail on wording. What matters is that neither
 * silently accepted something the other refused.
 */
function compare(mine, theirs, entry) {
  if (!mine) return 'harness: the probe returned no result for this case'
  if (!mine.ok && !theirs.ok) return null
  if (mine.ok && !theirs.ok) {
    return `mfb accepted, jose refused (${theirs.reason})\n    mfb:  ${canonical(answerOf(mine, entry))}`
  }
  if (!mine.ok && theirs.ok) {
    return `jose accepted, mfb refused ([${mine.code}] ${mine.message})\n    jose: ${canonical(answerOf(theirs, entry))}`
  }
  const a = canonical(answerOf(mine, entry))
  const b = canonical(answerOf(theirs, entry))
  if (a === b) return null
  return `both accepted, answers differ\n    mfb:  ${a}\n    jose: ${b}`
}

/** The members of a JWK that are the KEY, as opposed to how it is labelled. */
const KEY_MATERIAL = ['kty', 'crv', 'x', 'y', 'd', 'k']

/**
 * The part of an answer worth comparing.
 *
 * `id` is bookkeeping, and a verify answer's `header` is dropped because both
 * sides just echo the token's own header back — comparing it would compare
 * `JSON.parse` against `json::parse`, which is a different package's problem.
 */
function answerOf(result, entry) {
  if (result.token !== undefined) {
    // ECDSA signing is randomized (`mfb man crypto sign`), so two signatures of
    // one message are both valid and never equal. The `cross` mode is what
    // checks an `ES*` token, by handing it to the other side's verifier; here
    // the only comparable fact is that both sides produced one.
    const curve = entry?.key?.crv
    if (typeof curve === 'string' && curve.startsWith('P-')) return { signed: true }
    return { token: result.token }
  }
  if (result.claims !== undefined) return { claims: result.claims, algorithm: result.algorithm, kid: result.kid }
  if (result.jwk !== undefined) {
    // Compare the key, not its labelling. `jwt::exportJwk` writes `alg` and
    // `use: "sig"` and keeps the `kid`; jose's `exportJWK` writes the bare key
    // parameters. Both are legal JWKs, and neither policy is the other's bug —
    // what has to match is the material the two read out of one JWK.
    const material = {}
    for (const name of KEY_MATERIAL) {
      if (result.jwk[name] !== undefined) material[name] = result.jwk[name]
    }
    return { algorithm: result.algorithm, curve: result.curve, jwk: material }
  }
  return {}
}

/** Run one job through both sides and report each case's verdict. */
async function runBoth(job) {
  const mine = runProbe(job)
  if (mine.broken) return { broken: mine.broken }
  const theirs = await runJob(job)
  const byId = new Map(mine.results.map((r) => [r.id, r]))
  return {
    verdicts: job.cases.map((entry, index) => ({
      id: entry.id,
      entry,
      mine: byId.get(entry.id),
      theirs: theirs.results[index],
      difference: compare(byId.get(entry.id), theirs.results[index], entry),
    })),
  }
}

/**
 * Run a long job in batches.
 *
 * One process per batch rather than one per case (which would cost more than
 * answering it) and rather than one for everything (which puts a whole fuzzing
 * run behind a single timeout, and makes a failure name a job of thousands).
 * Ed448 and P-521 are software cores, so a few hundred signatures is already
 * seconds of real work.
 */
async function runBatches(entries, size = 250) {
  const verdicts = []
  for (let at = 0; at < entries.length; at += size) {
    const outcome = await runBoth({ cases: entries.slice(at, at + size) })
    if (outcome.broken) return { broken: `${outcome.broken} (cases ${at}..${at + size})` }
    verdicts.push(...outcome.verdicts)
  }
  return { verdicts }
}

/** Report a batch of verdicts against the declared divergences. */
function report(label, verdicts) {
  let failures = 0
  let declared = 0
  for (const verdict of verdicts) {
    const reason = DIVERGENCES[verdict.id]
    if (reason) {
      declared += 1
      if (!verdict.difference) {
        console.log(`FAIL ${verdict.id}: declared to diverge (${reason}) but AGREED`)
        console.log('     If the package now matches jose here, delete its divergences.json row.')
        failures += 1
      }
      continue
    }
    if (verdict.difference) {
      console.log(`FAIL ${verdict.id}: ${verdict.difference}`)
      failures += 1
    }
  }
  void label
  return { failures, declared }
}

// ---------------------------------------------------------------------------
// corpus
// ---------------------------------------------------------------------------

async function modeCorpus() {
  const entries = cases()
  const outcome = await runBoth({ cases: entries })
  if (outcome.broken) {
    console.log(`FAIL corpus: ${outcome.broken}`)
    return { cases: entries.length, failures: 1 }
  }
  const { failures, declared } = report('corpus', outcome.verdicts)
  return { cases: entries.length, failures, note: `${declared} declared divergence(s)` }
}

// ---------------------------------------------------------------------------
// cross — each side signs, the other verifies
// ---------------------------------------------------------------------------

async function modeCross() {
  const claims = { iss: 'https://issuer.example', sub: 'user-1', exp: NOW + 3600 }
  const options = { algorithms: ALGORITHMS, timeSeconds: NOW }

  // 1. Both sides sign the same claims under the same key.
  const signJob = {
    cases: SIGNING_KEYS.map((name) => ({ id: `cross-sign/${name}`, op: 'sign', key: KEYS[name], claims })),
  }
  const signed = await runBoth(signJob)
  if (signed.broken) {
    console.log(`FAIL cross: ${signed.broken}`)
    return { cases: 0, failures: 1 }
  }

  let failures = 0
  let checks = 0

  // 2. Each side's token goes to the OTHER side's verifier. This is what makes
  //    the mode worth running: a signature format only we produce would still
  //    round-trip through our own verifier.
  const crossCases = []
  for (const verdict of signed.verdicts) {
    const name = verdict.id.slice('cross-sign/'.length)
    if (!verdict.mine?.ok || !verdict.theirs?.ok) {
      console.log(`FAIL ${verdict.id}: one side could not sign at all — mfb ${verdict.mine?.ok}, jose ${verdict.theirs?.ok}`)
      if (verdict.mine && !verdict.mine.ok) console.log(`     mfb:  [${verdict.mine.code}] ${verdict.mine.message}`)
      if (verdict.theirs && !verdict.theirs.ok) console.log(`     jose: ${verdict.theirs.reason}`)
      failures += 1
      continue
    }
    crossCases.push({
      id: `cross-verify/${name}/jose-signed`,
      op: 'verify',
      token: verdict.theirs.token,
      keys: [publicOf(KEYS[name])],
      options,
    })
    // The deterministic algorithms must produce the SAME token. HMAC and RFC
    // 8032 EdDSA are functions of key and message alone; ECDSA is randomized,
    // so it is compared only by cross-verification.
    checks += 1
    if (!name.startsWith('ES') && verdict.mine.token !== verdict.theirs.token) {
      console.log(`FAIL cross-sign/${name}: deterministic algorithm, different tokens`)
      console.log(`     mfb:  ${verdict.mine.token}`)
      console.log(`     jose: ${verdict.theirs.token}`)
      failures += 1
    }
  }

  // 3. And the probe's tokens through jose.
  const probeVerify = await runBoth({ cases: crossCases })
  if (probeVerify.broken) {
    console.log(`FAIL cross: ${probeVerify.broken}`)
    return { cases: checks, failures: failures + 1 }
  }
  const reported = report('cross', probeVerify.verdicts)
  failures += reported.failures
  checks += probeVerify.verdicts.length

  for (const verdict of signed.verdicts) {
    const name = verdict.id.slice('cross-sign/'.length)
    if (!verdict.mine?.ok) continue
    const job = {
      cases: [
        {
          id: `cross-verify/${name}/mfb-signed`,
          op: 'verify',
          token: verdict.mine.token,
          keys: [publicOf(KEYS[name])],
          options,
        },
      ],
    }
    const back = await runBoth(job)
    checks += 1
    if (back.broken) {
      console.log(`FAIL cross-verify/${name}: ${back.broken}`)
      failures += 1
      continue
    }
    failures += report('cross', back.verdicts).failures
  }

  return { cases: checks, failures }
}

// ---------------------------------------------------------------------------
// fuzz
// ---------------------------------------------------------------------------

/** A deterministic generator, so a failure is reproducible from its seed. */
function makeRandom(seed) {
  let state = seed >>> 0
  return () => {
    state = (state * 1664525 + 1013904223) >>> 0
    return state / 4294967296
  }
}

const CLAIM_NAMES = ['sub', 'iss', 'aud', 'jti', 'scope', 'name', 'groups', 'email', 'nonce', 'act']

function randomValue(random, depth = 0) {
  const roll = random()
  if (depth > 2 || roll < 0.55) {
    const pick = random()
    if (pick < 0.4) return ['alpha', '', 'a b c', 'café 中文 😀', 'a"quote', 'back\\slash', ' '][Math.floor(random() * 7)]
    if (pick < 0.6) return Math.floor(random() * 2e9) - 1e9
    if (pick < 0.75) return Math.round(random() * 1e6) / 1e3
    if (pick < 0.9) return random() < 0.5
    return null
  }
  if (roll < 0.8) {
    const items = []
    for (let i = 0; i < Math.floor(random() * 4); i += 1) items.push(randomValue(random, depth + 1))
    return items
  }
  const object = {}
  for (let i = 0; i < Math.floor(random() * 4); i += 1) {
    object[`k${i}`] = randomValue(random, depth + 1)
  }
  return object
}

function randomClaims(random) {
  const claims = {}
  for (const name of CLAIM_NAMES) {
    if (random() < 0.4) claims[name] = randomValue(random)
  }
  // Time claims stay well-formed NumericDates: a fuzzed `exp` of 1.5 is a case
  // the corpus already pins deliberately, and letting it recur here would bury
  // everything else under one known divergence.
  if (random() < 0.6) claims.exp = NOW + Math.floor(random() * 7200) - 3600
  if (random() < 0.4) claims.nbf = NOW - Math.floor(random() * 3600)
  if (random() < 0.4) claims.iat = NOW - Math.floor(random() * 3600)
  return claims
}

function randomOptions(random, algorithm, claims) {
  const options = { algorithms: [algorithm], timeSeconds: NOW }
  if (random() < 0.3) options.leewaySeconds = Math.floor(random() * 120)
  if (random() < 0.2) options.maxAgeSeconds = Math.floor(random() * 7200) + 1
  if (random() < 0.3 && typeof claims.iss === 'string') options.issuer = claims.iss
  if (random() < 0.2) options.issuer = 'https://nobody.example'
  if (random() < 0.3 && typeof claims.sub === 'string') options.subject = claims.sub
  if (random() < 0.3 && typeof claims.aud === 'string') options.audience = claims.aud
  if (random() < 0.2) options.requiredClaims = ['exp']
  return options
}

async function modeFuzz(count) {
  const random = makeRandom(20260906)
  const entries = []
  for (let index = 0; index < count; index += 1) {
    const name = SIGNING_KEYS[index % SIGNING_KEYS.length]
    const algorithm = name.startsWith('EdDSA') ? 'EdDSA' : name
    const claims = randomClaims(random)
    // jose's signer needs a plain object; a token with no claims at all is
    // still a JWT, so the empty case is deliberately reachable.
    entries.push({ id: `fuzz/${index}/sign`, op: 'sign', key: KEYS[name], claims })
    entries.push({
      id: `fuzz/${index}/verify`,
      op: 'verify',
      token: token(name, claims),
      keys: [publicOf(KEYS[name])],
      options: randomOptions(random, algorithm, claims),
    })
  }
  const outcome = await runBatches(entries)
  if (outcome.broken) {
    console.log(`FAIL fuzz: ${outcome.broken}`)
    return { cases: entries.length, failures: 1 }
  }
  let failures = 0
  for (const verdict of outcome.verdicts) {
    if (!verdict.difference) continue
    console.log(`FAIL ${verdict.id}: ${verdict.difference}`)
    console.log(`     case: ${JSON.stringify(verdict.entry).slice(0, 400)}`)
    failures += 1
    if (failures > 10) {
      console.log('...stopping after 10 fuzz failures')
      break
    }
  }
  return { cases: entries.length, failures }
}

// ---------------------------------------------------------------------------
// mutate
// ---------------------------------------------------------------------------

const DAMAGE = ['.', '=', '+', '/', 'A', 'z', '0', '_', '-', '', ' ', '\n', 'é']

function mutate(text, random) {
  // Loop until the text actually changed. `DAMAGE` holds the empty string (so a
  // deletion is reachable), and splicing it in changes nothing — an unchanged
  // "mutation" that both sides verify would inflate the still-verified count
  // with cases that were never damaged.
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const characters = [...text]
    const edits = 1 + Math.floor(random() * 3)
    for (let i = 0; i < edits && characters.length > 0; i += 1) {
      const at = Math.floor(random() * characters.length)
      const roll = random()
      if (roll < 0.4) characters[at] = DAMAGE[Math.floor(random() * DAMAGE.length)]
      else if (roll < 0.7) characters.splice(at, 1)
      else characters.splice(at, 0, DAMAGE[Math.floor(random() * DAMAGE.length)])
    }
    const damaged = characters.join('')
    if (damaged !== text) return damaged
  }
  return `${text}x`
}

/**
 * Whether `tokenText` is a NON-CANONICAL spelling: at least one segment decodes
 * to bytes whose canonical unpadded Base64url is a different string.
 *
 * This is what turns "jose accepted what we refused" from an excuse into a
 * checked claim. `packages/jwt` refuses a segment that is not canonical
 * (RFC 7515 §2), and jose's decoder is `Buffer.from(…, 'base64url')`, which
 * ignores padding, whitespace and a trailing orphan symbol — so damage that only
 * changes the SPELLING of a segment still verifies there. Re-encoding jose's own
 * decode and comparing proves that is what happened: if the text round-trips
 * unchanged, the token was canonical and our refusal is a bug, not a policy.
 */
function isNonCanonicalSpelling(tokenText) {
  const parts = tokenText.split('.')
  if (parts.length !== 3) return false
  return parts.some((part) => {
    try {
      return base64url.encode(base64url.decode(part)) !== part
    } catch {
      return false
    }
  })
}

async function modeMutate(count) {
  const random = makeRandom(557)
  const sources = SIGNING_KEYS.map((name) => ({
    name,
    token: token(name, { iss: 'https://issuer.example', sub: 'user-1', exp: NOW + 3600 }),
  }))
  const entries = []
  for (let index = 0; index < count; index += 1) {
    const source = sources[index % sources.length]
    entries.push({
      id: `mutate/${index}`,
      op: 'verify',
      token: mutate(source.token, random),
      keys: [publicOf(KEYS[source.name])],
      options: { algorithms: ALGORITHMS, timeSeconds: NOW },
    })
  }
  const outcome = await runBatches(entries)
  if (outcome.broken) {
    // The whole point of the mode: damage must not break the probe.
    console.log(`FAIL mutate: ${outcome.broken}`)
    return { cases: entries.length, failures: 1 }
  }
  let failures = 0
  let accepted = 0
  let lenient = 0
  for (const verdict of outcome.verdicts) {
    if (verdict.mine?.ok) accepted += 1
    if (!verdict.difference) continue
    // The one difference this mode expects: damage that only changes a
    // segment's SPELLING. jose decodes it back to the same bytes and verifies;
    // `packages/jwt` refuses the text. Counted rather than failed — but only
    // once the token has been checked to actually be a non-canonical spelling.
    if (!verdict.mine.ok && verdict.theirs.ok && isNonCanonicalSpelling(verdict.entry.token)) {
      lenient += 1
      continue
    }
    console.log(`FAIL ${verdict.id}: ${verdict.difference}`)
    console.log(`     token: ${JSON.stringify(verdict.entry.token).slice(0, 200)}`)
    failures += 1
    if (failures > 10) {
      console.log('...stopping after 10 mutate failures')
      break
    }
  }
  return {
    cases: entries.length,
    failures,
    // Both counts are information, not verdicts. A mutation that still verifies
    // landed somewhere the signature does not cover; a lenient one is a
    // non-canonical spelling jose read through and we did not.
    note: `${accepted} still verified, ${lenient} non-canonical spelling(s) jose read and mfb refused`,
  }
}

// ---------------------------------------------------------------------------

const MODES = {
  corpus: () => modeCorpus(),
  cross: () => modeCross(),
  fuzz: (n) => modeFuzz(n),
  mutate: (n) => modeMutate(n),
}

function usage(problem) {
  process.stderr.write(`${problem}\nusage: node diff.mjs [corpus|cross|fuzz|mutate ...] [--count N]\n`)
  return 2
}

async function main(argv) {
  const countAt = argv.indexOf('--count')
  let count = 300
  if (countAt >= 0) {
    count = Number(argv[countAt + 1])
    argv.splice(countAt, 2)
    if (!Number.isInteger(count) || count < 1) return usage('--count needs a positive integer')
  }
  const wanted = argv.length > 0 ? argv : Object.keys(MODES)
  const unknown = wanted.filter((name) => !(name in MODES))
  if (unknown.length > 0) return usage(`unknown mode(s) ${unknown.join(', ')}`)

  try {
    execFileSync(PROBE, [], { stdio: 'ignore' })
  } catch (problem) {
    if (problem.code === 'ENOENT') {
      process.stderr.write(
        `${PROBE} is missing. Build it first:\n` +
          '  mfb build packages/jwt\n' +
          '  cp packages/jwt/jwt.mfp packages/jwt/oracle/probe/packages/jwt.mfp\n' +
          '  mfb build packages/jwt/oracle/probe\n',
      )
      return 1
    }
    // A non-zero exit from a missing argument is fine — the binary exists.
  }

  let total = 0
  for (const name of wanted) {
    const { cases: seen, failures, note } = await MODES[name](count)
    total += failures
    console.log(`${name}: ${seen} case(s), ${failures} failure(s)${note ? ` — ${note}` : ''}`)
  }
  if (total === 0) {
    console.log('\nEvery case agreed, or diverged for a declared reason (divergences.json).')
  }
  return total === 0 ? 0 : 1
}

void craft
process.exit(await main(process.argv.slice(2)))
