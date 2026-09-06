// corpus.mjs — the hand-written cases, as job entries both sides answer.
//
// Each one is a whole realistic question rather than a snippet, so a failure
// names a RULE ("aud may be an array") instead of a line number. The cases that
// must NOT agree live here too, declared in divergences.json with the reason —
// a declared case that stops diverging fails the run, which is how the harness
// notices the package quietly changing a documented policy.
//
// Keys come from keys.json, which is committed: a corpus case has to be
// reproducible by hand, and a freshly generated key makes yesterday's failure
// unrepeatable. They are test keys and nothing else has ever signed with them.

import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createHmac, createPrivateKey, sign as nodeSign } from 'node:crypto'
import { base64url } from 'jose'

const HERE = dirname(fileURLToPath(import.meta.url))
export const KEYS = JSON.parse(readFileSync(join(HERE, 'keys.json'), 'utf8'))

/** The instant every case is judged at, so no case depends on today. */
export const NOW = 1700000000

export const SIGNING_KEYS = Object.keys(KEYS)

/** A key with its private half removed — what a verifier would be handed. */
export function publicOf(jwk) {
  if (jwk.kty === 'oct') return { ...jwk }
  const { d, ...rest } = jwk
  void d
  return rest
}

function options(extra = {}) {
  return { algorithms: [...new Set(SIGNING_KEYS.map(algOf))], timeSeconds: NOW, ...extra }
}

function algOf(name) {
  return name.startsWith('EdDSA') ? 'EdDSA' : name
}

/** A compact JWS over exactly the header and claims TEXT given, signed for real. */
export function craft(keyName, headerJson, payloadJson) {
  const jwk = KEYS[keyName]
  const input = `${base64url.encode(headerJson)}.${base64url.encode(payloadJson)}`
  if (jwk.kty === 'oct') {
    const bits = { HS256: 'sha256', HS384: 'sha384', HS512: 'sha512' }[jwk.alg]
    const mac = createHmac(bits, Buffer.from(jwk.k, 'base64url')).update(input).digest()
    return `${input}.${base64url.encode(mac)}`
  }
  const key = createPrivateKey({ key: jwk, format: 'jwk' })
  const signature = nodeSign(null, Buffer.from(input), { key, dsaEncoding: 'ieee-p1363' })
  return `${input}.${base64url.encode(signature)}`
}

/** Sign `claims` with `keyName`, through the same path a good token takes. */
export function token(keyName, claims, header = {}) {
  const jwk = KEYS[keyName]
  const full = { alg: algOf(keyName), typ: 'JWT', ...(jwk.kid ? { kid: jwk.kid } : {}), ...header }
  return craft(keyName, JSON.stringify(full), JSON.stringify(claims))
}

/**
 * Every corpus case. Each is a job entry plus an `id` that `divergences.json`
 * keys on.
 */
export function cases() {
  const entries = []
  const add = (id, entry) => entries.push({ id, ...entry })

  // -------------------------------------------------------------------------
  // Signing: every algorithm, and the header the package writes.
  // -------------------------------------------------------------------------
  for (const name of SIGNING_KEYS) {
    add(`sign/${name}`, {
      op: 'sign',
      key: KEYS[name],
      claims: { iss: 'https://issuer.example', sub: 'user-1', iat: NOW - 60, exp: NOW + 3600 },
    })
    add(`sign/${name}/extra-header`, {
      op: 'sign',
      key: KEYS[name],
      header: { typ: 'at+jwt', cty: 'JWT' },
      claims: { sub: 'user-1' },
    })
  }

  // -------------------------------------------------------------------------
  // Verifying a good token, on every algorithm and against the public half.
  // -------------------------------------------------------------------------
  for (const name of SIGNING_KEYS) {
    const claims = { iss: 'https://issuer.example', aud: 'https://api.example', sub: 'user-1', exp: NOW + 3600 }
    add(`verify/${name}`, {
      op: 'verify',
      token: token(name, claims),
      keys: [publicOf(KEYS[name])],
      options: options({ issuer: 'https://issuer.example', audience: 'https://api.example' }),
    })
  }

  // A verifier holding every key at once still picks the right one by `kid`.
  add('verify/whole-key-set', {
    op: 'verify',
    token: token('ES384', { sub: 'user-1' }),
    keys: SIGNING_KEYS.map((name) => publicOf(KEYS[name])),
    options: options(),
  })

  // -------------------------------------------------------------------------
  // Claims: the registered ones, at and around their boundaries.
  // -------------------------------------------------------------------------
  const claimCases = [
    ['exp/future', { exp: NOW + 1 }, {}],
    ['exp/now', { exp: NOW }, {}],
    ['exp/past', { exp: NOW - 1 }, {}],
    ['exp/past-within-leeway', { exp: NOW - 30 }, { leewaySeconds: 60 }],
    ['exp/past-outside-leeway', { exp: NOW - 90 }, { leewaySeconds: 60 }],
    ['nbf/now', { nbf: NOW }, {}],
    ['nbf/future', { nbf: NOW + 1 }, {}],
    ['nbf/future-within-leeway', { nbf: NOW + 30 }, { leewaySeconds: 60 }],
    ['iat/uncompared-future', { iat: NOW + 86400 }, {}],
    ['iat/within-max-age', { iat: NOW - 60 }, { maxAgeSeconds: 300 }],
    ['iat/past-max-age', { iat: NOW - 600 }, { maxAgeSeconds: 300 }],
    ['iat/future-under-max-age', { iat: NOW + 600 }, { maxAgeSeconds: 300 }],
    ['iat/absent-under-max-age', { sub: 'a' }, { maxAgeSeconds: 300 }],
    ['iss/match', { iss: 'https://issuer.example' }, { issuer: 'https://issuer.example' }],
    ['iss/mismatch', { iss: 'https://other.example' }, { issuer: 'https://issuer.example' }],
    ['iss/absent', { sub: 'a' }, { issuer: 'https://issuer.example' }],
    ['sub/match', { sub: 'user-1' }, { subject: 'user-1' }],
    ['sub/mismatch', { sub: 'user-2' }, { subject: 'user-1' }],
    ['aud/string-match', { aud: 'https://api.example' }, { audience: 'https://api.example' }],
    ['aud/string-mismatch', { aud: 'https://other.example' }, { audience: 'https://api.example' }],
    ['aud/array-match', { aud: ['a', 'https://api.example', 'c'] }, { audience: 'https://api.example' }],
    ['aud/array-mismatch', { aud: ['a', 'b'] }, { audience: 'https://api.example' }],
    ['aud/absent', { sub: 'a' }, { audience: 'https://api.example' }],
    ['required/present', { sub: 'a', exp: NOW + 60 }, { requiredClaims: ['sub', 'exp'] }],
    ['required/absent', { sub: 'a' }, { requiredClaims: ['sub', 'exp'] }],
    ['typ/required-match', { sub: 'a' }, { typeHeader: 'JWT' }],
    ['typ/required-mismatch', { sub: 'a' }, { typeHeader: 'at+jwt' }],
    ['numericdate/fractional-exp', { exp: NOW + 0.5 }, {}],
    ['numericdate/string-exp', { exp: `${NOW + 60}` }, {}],
    ['numericdate/huge-exp', { exp: 1e19 }, {}],
    ['aud/not-a-string', { aud: 12 }, { audience: 'a' }],
  ]
  for (const [id, claims, extra] of claimCases) {
    add(`claims/${id}`, {
      op: 'verify',
      token: token('HS256', claims),
      keys: [KEYS.HS256],
      options: options(extra),
    })
  }

  // -------------------------------------------------------------------------
  // Rejections: the shapes a verifier must refuse.
  // -------------------------------------------------------------------------
  const good = token('HS256', { sub: 'user-1' })
  const [head, body, mac] = good.split('.')
  const reject = (id, tokenText, extra = {}, keys = [KEYS.HS256]) =>
    add(`reject/${id}`, { op: 'verify', token: tokenText, keys, options: options(extra) })

  reject('empty', '')
  reject('two-segments', `${head}.${body}`)
  reject('four-segments', `${good}.extra`)
  reject('jwe-shape', 'a.b.c.d.e')
  reject('empty-header', `.${body}.${mac}`)
  reject('empty-payload', `${head}..${mac}`)
  reject('empty-signature', `${head}.${body}.`)
  reject('tampered-payload', `${head}.${base64url.encode(JSON.stringify({ sub: 'admin' }))}.${mac}`)
  reject('tampered-signature', `${head}.${body}.${base64url.encode(Buffer.alloc(32, 7))}`)
  reject('truncated-signature', `${head}.${body}.${mac.slice(0, 10)}`)
  reject('wrong-key', good, {}, [{ ...KEYS.HS256, k: base64url.encode(Buffer.alloc(32, 9)), kid: 'other' }])
  reject('no-key-for-alg', good, {}, [publicOf(KEYS.ES256)])
  reject('alg-not-allowed', good, { algorithms: ['ES256'] })
  reject('unsecured-none', `${base64url.encode('{"alg":"none"}')}.${body}.`)
  reject('unsecured-none-signed', craft('HS256', '{"alg":"none","typ":"JWT"}', '{"sub":"admin"}'))
  reject('header-not-an-object', craft('HS256', '["HS256"]', '{"sub":"a"}'))
  reject('payload-not-an-object', craft('HS256', '{"alg":"HS256"}', '[1,2,3]'))
  reject('header-not-json', craft('HS256', 'not json', '{"sub":"a"}'))
  reject('no-alg', craft('HS256', '{"typ":"JWT"}', '{"sub":"a"}'))
  reject('alg-not-a-string', craft('HS256', '{"alg":256}', '{"sub":"a"}'))
  reject('crit', craft('HS256', '{"alg":"HS256","crit":["exp"],"exp":1}', '{"sub":"a"}'))
  reject('rs256-unsupported', craft('HS256', '{"alg":"RS256","typ":"JWT"}', '{"sub":"a"}'), {
    algorithms: ['HS256', 'RS256'],
  })

  // Algorithm confusion: an HS256 token keyed by the ES256 PUBLIC key everyone
  // already has. Both the allowlist and the key's own algorithm refuse it.
  const publicAsSecret = base64url.encode(
    Buffer.concat([Buffer.from([4]), Buffer.from(KEYS.ES256.x, 'base64url'), Buffer.from(KEYS.ES256.y, 'base64url')]),
  )
  add('reject/algorithm-confusion', {
    op: 'verify',
    token: craft('HS256', '{"alg":"HS256","typ":"JWT"}', '{"sub":"admin"}'),
    keys: [publicOf(KEYS.ES256), { kty: 'oct', k: publicAsSecret, alg: 'HS256' }],
    options: options({ algorithms: ['ES256'] }),
  })

  // -------------------------------------------------------------------------
  // Base64url canonicality and duplicate members. `packages/jwt` is stricter
  // than jose here, deliberately; each is declared in divergences.json.
  // -------------------------------------------------------------------------
  add('strict/padded-payload', {
    op: 'verify',
    // Signed over the PADDED text, so the signature is not what refuses it.
    token: (() => {
      const padded = `${base64url.encode('{"alg":"HS256","typ":"JWT"}')}.${base64url.encode('{"sub":"a"}')}=`
      const mac = createHmac('sha256', Buffer.from(KEYS.HS256.k, 'base64url')).update(padded).digest()
      return `${padded}.${base64url.encode(mac)}`
    })(),
    keys: [KEYS.HS256],
    options: options(),
  })
  add('strict/duplicate-alg', {
    op: 'verify',
    token: craft('HS256', '{"alg":"HS256","alg":"HS256","typ":"JWT"}', '{"sub":"a"}'),
    keys: [KEYS.HS256],
    options: options(),
  })
  add('strict/duplicate-exp', {
    op: 'verify',
    token: craft('HS256', '{"alg":"HS256","typ":"JWT"}', `{"exp":${NOW - 1},"exp":${NOW + 3600}}`),
    keys: [KEYS.HS256],
    options: options(),
  })
  add('strict/short-hmac-secret', {
    op: 'verify',
    token: (() => {
      const short = Buffer.alloc(8, 3)
      const input = `${base64url.encode('{"alg":"HS256","typ":"JWT"}')}.${base64url.encode('{"sub":"a"}')}`
      const mac = createHmac('sha256', short).update(input).digest()
      return `${input}.${base64url.encode(mac)}`
    })(),
    keys: [{ kty: 'oct', k: base64url.encode(Buffer.alloc(8, 3)), alg: 'HS256' }],
    options: options(),
  })

  // A nested object may repeat a name; only the TOP level of a header or a
  // claims set is ambiguous. This one must agree.
  add('strict/nested-repeat-is-fine', {
    op: 'verify',
    token: craft('HS256', '{"alg":"HS256","typ":"JWT"}', '{"a":{"alg":1},"b":{"alg":2}}'),
    keys: [KEYS.HS256],
    options: options(),
  })

  // -------------------------------------------------------------------------
  // JWK import/export.
  // -------------------------------------------------------------------------
  for (const name of SIGNING_KEYS) {
    // A MAC key has no public form to ask for — the secret IS the key — so only
    // the private direction is a question either side can answer.
    if (KEYS[name].kty !== 'oct') {
      add(`jwk/${name}/public`, { op: 'jwk', key: publicOf(KEYS[name]), withPrivate: false })
    }
    add(`jwk/${name}/private`, { op: 'jwk', key: KEYS[name], withPrivate: true })
  }
  add('jwk/rsa-unsupported', { op: 'jwk', key: { kty: 'RSA', n: 'AQAB', e: 'AQAB' }, withPrivate: false })
  add('jwk/short-coordinate', {
    op: 'jwk',
    key: { ...publicOf(KEYS.ES256), x: base64url.encode(Buffer.alloc(31, 1)) },
    withPrivate: false,
  })
  add('jwk/marked-for-encryption', { op: 'jwk', key: { ...publicOf(KEYS.ES256), use: 'enc' }, withPrivate: false })
  add('jwk/oct-without-alg', {
    op: 'jwk',
    key: { kty: 'oct', k: KEYS.HS256.k },
    withPrivate: true,
  })

  return entries
}
