#!/usr/bin/env node
// oracle.mjs — answer a job file of JWT sign/verify/jwk cases with `jose`, in
// exactly the shape `probe/` answers it.
//
//     node oracle.mjs <job.json>
//
// Why THIS module: `jose` (panva/jose) is the JOSE implementation the Node
// ecosystem actually runs. It is written from the same RFCs `packages/jwt` is
// written from, by someone else, which is the whole point — the package's own
// tests pin what this project DECIDED, and they cannot catch a misreading of
// RFC 7515/7518/7519 that the code and the tests share.
//
// Why version 5 and not 6: jose 6 dropped `EdDSA` over Ed448, because it runs on
// WebCrypto and WebCrypto has no Ed448. `packages/jwt` supports both RFC 8037
// curves (`crypto` implements Ed448 in software), so an oracle without Ed448
// would leave a third of the signature surface unchecked. jose 5 reaches Node's
// own `crypto`, which has it.
//
// The job/answer protocol is `probe/src/main.mfb`'s, described there. Keys cross
// as JWKs because that is the one key format both sides speak.

import { readFileSync } from 'node:fs'
import {
  SignJWT,
  jwtVerify,
  importJWK,
  exportJWK,
  base64url,
  decodeProtectedHeader,
} from 'jose'
import { createPrivateKey, createPublicKey } from 'node:crypto'

/** The seven algorithms `packages/jwt` implements. */
export const ALGORITHMS = ['HS256', 'HS384', 'HS512', 'EdDSA', 'ES256', 'ES384', 'ES512']

/** The `alg` a JWK is for, the same way the package derives it. */
export function algorithmOf(jwk) {
  if (jwk.alg) return jwk.alg
  switch (jwk.crv) {
    case 'Ed25519':
    case 'Ed448':
      return 'EdDSA'
    case 'P-256':
      return 'ES256'
    case 'P-384':
      return 'ES384'
    case 'P-521':
      return 'ES512'
    default:
      return undefined
  }
}

/**
 * Import a JWK as a key jose will sign or verify with.
 *
 * `importJWK` refuses a private EC/OKP JWK for verification and a public one for
 * signing, so the caller says which half it wants. An `oct` key is the same
 * value either way.
 */
async function importKey(jwk, algorithm) {
  if (jwk.kty === 'oct') return importJWK(jwk, algorithm)
  // Node's own KeyObject path rather than `importJWK`, because jose 5 routes
  // Ed448 through it and this keeps every curve on one code path.
  const { kty, crv, x, y, d, ...rest } = jwk
  const bare = d ? { kty, crv, x, y, d } : { kty, crv, x, y }
  void rest
  return d ? createPrivateKey({ key: bare, format: 'jwk' }) : createPublicKey({ key: bare, format: 'jwk' })
}

/** jose's option object for one job `options` block. */
function verifyOptions(options = {}) {
  const built = { algorithms: options.algorithms ?? [] }
  if (options.issuer) built.issuer = options.issuer
  if (options.audience) built.audience = options.audience
  if (options.subject) built.subject = options.subject
  if (options.typeHeader) built.typ = options.typeHeader
  if (options.leewaySeconds) built.clockTolerance = options.leewaySeconds
  if (options.maxAgeSeconds) built.maxTokenAge = options.maxAgeSeconds
  if (options.requiredClaims?.length) built.requiredClaims = options.requiredClaims
  if (options.timeSeconds !== undefined) built.currentDate = new Date(options.timeSeconds * 1000)
  return built
}

async function runSign(entry) {
  const algorithm = algorithmOf(entry.key)
  if (!ALGORITHMS.includes(algorithm)) {
    throw new Error(`unsupported algorithm ${algorithm}`)
  }
  const key = await importKey(entry.key, algorithm)
  // The header the package writes, spelled in the package's order: `alg`, then
  // `typ`, then `kid`, then anything else. jose serializes the object as given,
  // so matching the order is what makes a byte-for-byte token comparison
  // possible for the deterministic algorithms.
  const header = { alg: algorithm }
  header.typ = entry.header?.typ ?? 'JWT'
  const kid = entry.header?.kid ?? entry.key.kid
  if (kid !== undefined) header.kid = kid
  for (const [name, value] of Object.entries(entry.header ?? {})) {
    if (name !== 'alg' && name !== 'typ' && name !== 'kid') header[name] = value
  }
  return { ok: true, token: await new SignJWT(entry.claims).setProtectedHeader(header).sign(key) }
}

async function runVerify(entry) {
  const options = verifyOptions(entry.options)
  const header = decodeProtectedHeader(entry.token)
  let lastProblem
  for (const jwk of entry.keys ?? []) {
    const algorithm = algorithmOf(jwk)
    // Mirror the package's key selection: only a key whose OWN algorithm is the
    // token's is tried, and a key with a `kid` only when the token asks for that
    // one or asks for nothing. Without this the comparison would be measuring
    // jose's key handling rather than the package's rules.
    if (algorithm !== header.alg) continue
    if (header.kid && jwk.kid && jwk.kid !== header.kid) continue
    let key
    try {
      key = await importKey(jwk, algorithm)
    } catch (problem) {
      lastProblem = problem
      continue
    }
    try {
      const result = await jwtVerify(entry.token, key, options)
      return {
        ok: true,
        header: result.protectedHeader,
        claims: result.payload,
        algorithm: result.protectedHeader.alg,
        kid: jwk.kid ?? '',
      }
    } catch (problem) {
      lastProblem = problem
      // A claim failure is the token's verdict, not this key's, so stop rather
      // than reporting it as "no key verified".
      if (problem.code === 'ERR_JWT_CLAIM_VALIDATION_FAILED' || problem.code === 'ERR_JWT_EXPIRED') {
        throw problem
      }
    }
  }
  throw lastProblem ?? new Error('no candidate key')
}

async function runJwk(entry) {
  const algorithm = algorithmOf(entry.key)
  if (!ALGORITHMS.includes(algorithm)) throw new Error(`unsupported algorithm ${algorithm}`)
  const key = await importKey(entry.key, algorithm)
  const jwk = entry.key.kty === 'oct' ? { ...entry.key } : await exportJWK(key)
  return {
    ok: true,
    algorithm,
    curve: entry.key.crv ?? '',
    kid: entry.key.kid ?? '',
    jwk,
  }
}

/** Answer one case, turning any failure into a reported refusal. */
export async function runCase(entry) {
  try {
    let answer
    if (entry.op === 'sign') answer = await runSign(entry)
    else if (entry.op === 'verify') answer = await runVerify(entry)
    else if (entry.op === 'jwk') answer = await runJwk(entry)
    else throw new Error(`unknown op ${entry.op}`)
    return { id: entry.id, ...answer }
  } catch (problem) {
    return {
      id: entry.id,
      ok: false,
      reason: `${problem.code ?? problem.name ?? 'Error'}: ${problem.message ?? problem}`,
    }
  }
}

/** Answer a whole job, in order. */
export async function runJob(job) {
  const results = []
  for (const entry of job.cases) results.push(await runCase(entry))
  return { results }
}

async function main(argv) {
  if (argv.length !== 1) {
    process.stderr.write('usage: node oracle.mjs <job.json>\n')
    return 2
  }
  const job = JSON.parse(readFileSync(argv[0], 'utf8'))
  process.stdout.write(JSON.stringify(await runJob(job)) + '\n')
  return 0
}

if (import.meta.url === `file://${process.argv[1]}`) {
  process.exit(await main(process.argv.slice(2)))
}

export { base64url }
