import type { KeyObject } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { createHash, generateKeyPairSync, sign } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, it } from 'vitest'
import { parse } from 'yaml'

function signed(domain: string, statement: Buffer, privateKey: KeyObject) {
  return sign(null, Buffer.concat([
    Buffer.from(`${domain}\0`),
    createHash('sha256').update(statement).digest(),
  ]), privateKey).toString('base64url')
}

function rootFixture(rootKeyId: string, signer: { publicKey: KeyObject }, keyId = 'signer-1', additionalKeys: Partial<{
  keyId: string
  publicKey: string
  notBefore: string
  notAfter: string
  status: string
}>[] = []) {
  const { publicKey, privateKey } = generateKeyPairSync('ed25519')
  const rootPublicKey = publicKey.export({ format: 'jwk' }).x!
  const key = {
    keyId,
    algorithm: 'Ed25519',
    publicKey: signer.publicKey.export({ format: 'jwk' }).x!,
    notBefore: '2026-01-01T00:00:00.000Z',
    notAfter: '2027-01-01T00:00:00.000Z',
    status: 'active',
  }
  return {
    version: 1,
    rootKeyId,
    rootPublicKey,
    fetchedAt: '2026-11-18T00:00:00.000Z',
    keys: [key, ...additionalKeys.map(extra => ({ ...key, ...extra }))].map((key) => {
      const statement = Buffer.from(JSON.stringify({ version: 1, rootKeyId, ...key }))
      return {
        ...key,
        statement: statement.toString('base64url'),
        rootSignature: signed('skilld-trusted-key-v1', statement, privateKey),
      }
    }),
  }
}

function preflightFixture() {
  const directory = mkdtempSync(join(tmpdir(), 'skilld-root-preflight-'))
  const workflow = parse(readFileSync('.github/workflows/release.yml', 'utf8'))
  const step = workflow.jobs['release-plan'].steps.find(
    (step: { name?: string }) => step.name === 'Require skilld.dev to serve both pinned roots',
  )
  const signer = generateKeyPairSync('ed25519')
  const primary = rootFixture('primary-root', signer)
  const backup = rootFixture('backup-root', signer)
  const fixture = JSON.parse(readFileSync('contracts/fixtures/v1/artifact-attestation.json', 'utf8'))
  const { statement: _statement, signature: _signature, ...claims } = fixture
  const statement = Buffer.from(JSON.stringify(claims))
  const attestation = {
    ...claims,
    statement: statement.toString('base64url'),
    signature: { algorithm: 'Ed25519', keyId: 'signer-1', value: signed('skilld-attestation-v1', statement, signer.privateKey) },
  }
  // Replace only HTTP. Execute the release gate and the CLI's Rust verifiers.
  writeFileSync(join(directory, 'curl.ts'), `#!/usr/bin/env -S node --experimental-strip-types
import { copyFileSync, existsSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
const args = process.argv.slice(2)
const path = new URL(args.at(-1)!).pathname
if (path === '/api/v1/resolutions') {
  const request = JSON.parse(readFileSync(args[args.indexOf('--data-binary') + 1]!.slice(1), 'utf8'))
  if (request.source.ref.value !== process.env.GITHUB_SHA || request.source.selector.path !== 'skills/skilld')
    process.exit(1)
}
const id = path.startsWith('/api/v1/resolutions') ? 'resolution' : path.split('/').at(-1)!
if (path === '/api/v1/resolutions' && existsSync(join(process.env.ROOT_FIXTURES!, 'pending'))) {
  writeFileSync(args[args.indexOf('--output') + 1]!, JSON.stringify({ state: 'pending', resolutionId: 'res_release_probe' }))
  process.stdout.write('202')
  process.exit(0)
}
copyFileSync(join(process.env.ROOT_FIXTURES!, id + '.json'), args[args.indexOf('--output') + 1]!)
process.stdout.write('200')
`, { mode: 0o755 })
  symlinkSync('curl.ts', join(directory, 'curl'))
  const save = (root: typeof primary) => writeFileSync(join(directory, `${root.rootKeyId}.json`), JSON.stringify(root))
  save(primary)
  save(backup)
  const resolution = {
    state: 'ready',
    resolutionId: 'res_release_probe',
    artifact: { artifactId: claims.artifactId, visibility: 'public', attestation },
  }
  const saveResolution = () => writeFileSync(join(directory, 'resolution.json'), JSON.stringify(resolution))
  saveResolution()
  return {
    primary,
    backup,
    signer,
    save,
    resolution,
    saveResolution,
    pending: () => writeFileSync(join(directory, 'pending'), ''),
    run: () => spawnSync('bash', ['-e', '-c', `${step.run}\nprintf 'native-build-reached'`], {
      encoding: 'utf8',
      env: {
        ...process.env,
        PATH: `${directory}:${process.env.PATH}`,
        ROOT_FIXTURES: directory,
        GITHUB_SHA: claims.source.commitSha,
        GITHUB_RUN_ID: '1',
        GITHUB_RUN_ATTEMPT: '1',
        SKILLD_ROOT_KEY_ID: primary.rootKeyId,
        SKILLD_ROOT_PUBLIC_KEY: primary.rootPublicKey,
        SKILLD_BACKUP_ROOT_KEY_ID: backup.rootKeyId,
        SKILLD_BACKUP_ROOT_PUBLIC_KEY: backup.rootPublicKey,
      },
    }),
    cleanup: () => rmSync(directory, { recursive: true, force: true }),
  }
}

it.each(['primary', 'backup'] as const)('rejects an overlapping signer present only under the %s root despite a valid probe', (pin) => {
  const fixture = preflightFixture()
  try {
    const root = rootFixture(fixture[pin].rootKeyId, fixture.signer, 'signer-1', [{
      keyId: 'signer-2',
      publicKey: generateKeyPairSync('ed25519').publicKey.export({ format: 'jwk' }).x!,
      status: 'overlapping',
    }])
    Object.assign(fixture[pin], root)
    fixture.save(root)
    const result = fixture.run()
    expect(result.status, result.stderr).not.toBe(0)
    expect(result.stdout).not.toContain('native-build-reached')
    expect(result.stderr).toContain('supported Artifact signers differ')
  }
  finally {
    fixture.cleanup()
  }
}, 120_000)

it.each(['public-key', 'start', 'end', 'retired', 'revoked'] as const)('rejects different %s acceptance for a signer outside the probe', (difference) => {
  const fixture = preflightFixture()
  try {
    const extra = {
      keyId: 'signer-2',
      publicKey: generateKeyPairSync('ed25519').publicKey.export({ format: 'jwk' }).x!,
      status: 'overlapping',
    }
    const changed = {
      ...extra,
      ...(difference === 'public-key' ? { publicKey: fixture.signer.publicKey.export({ format: 'jwk' }).x! } : {}),
      ...(difference === 'start' ? { notBefore: '2026-06-01T00:00:00Z' } : {}),
      ...(difference === 'end' ? { notAfter: '2026-12-01T00:00:00Z' } : {}),
      ...(['retired', 'revoked'].includes(difference) ? { status: difference } : {}),
    }
    for (const pin of ['primary', 'backup'] as const) {
      const root = rootFixture(fixture[pin].rootKeyId, fixture.signer, 'signer-1', [pin === 'primary' ? extra : changed])
      Object.assign(fixture[pin], root)
      fixture.save(root)
    }
    const result = fixture.run()
    expect(result.status, result.stderr).not.toBe(0)
    expect(result.stdout).not.toContain('native-build-reached')
    expect(result.stderr).toContain('supported Artifact signers differ')
  }
  finally {
    fixture.cleanup()
  }
}, 120_000)

it('accepts equivalent signer policies regardless of key order, accepted status, or timestamp format', () => {
  const fixture = preflightFixture()
  try {
    const extra = {
      keyId: 'signer-2',
      publicKey: generateKeyPairSync('ed25519').publicKey.export({ format: 'jwk' }).x!,
      status: 'overlapping',
    }
    const primary = rootFixture(fixture.primary.rootKeyId, fixture.signer, 'signer-1', [extra, { keyId: 'retired', status: 'retired' }])
    const backup = rootFixture(fixture.backup.rootKeyId, fixture.signer, 'signer-1', [{
      ...extra,
      status: 'active',
      notBefore: '2026-01-01T01:00:00+01:00',
      notAfter: '2027-01-01T00:00:00Z',
    }])
    backup.keys.reverse()
    for (const root of [primary, backup]) {
      Object.assign(root.rootKeyId === primary.rootKeyId ? fixture.primary : fixture.backup, root)
      fixture.save(root)
    }
    const result = fixture.run()
    expect(result.status, result.stderr).toBe(0)
    expect(result.stdout).toContain('native-build-reached')
  }
  finally {
    fixture.cleanup()
  }
}, 120_000)

it.each(['primary', 'backup'] as const)('rejects a valid %s root whose signer cannot verify the service attestation', (pin) => {
  const fixture = preflightFixture()
  try {
    const incompatible = rootFixture(fixture[pin].rootKeyId, generateKeyPairSync('ed25519'))
    Object.assign(fixture[pin], incompatible)
    fixture.save(incompatible)
    const result = fixture.run()
    expect(result.status, result.stderr).not.toBe(0)
    expect(result.stdout).not.toContain('native-build-reached')
    expect(result.stderr).toContain('ATTESTATION_SIGNATURE_INVALID')
  }
  finally {
    fixture.cleanup()
  }
}, 120_000)

it.each(['primary', 'backup'] as const)('rejects an unknown service signer under the %s root', (pin) => {
  const fixture = preflightFixture()
  try {
    const incompatible = rootFixture(fixture[pin].rootKeyId, generateKeyPairSync('ed25519'), 'other-signer')
    Object.assign(fixture[pin], incompatible)
    fixture.save(incompatible)
    const result = fixture.run()
    expect(result.status, result.stderr).not.toBe(0)
    expect(result.stderr).toContain('SIGNING_KEY_UNKNOWN')
  }
  finally {
    fixture.cleanup()
  }
}, 120_000)

it('waits for a pending service attestation before verifying both roots', () => {
  const fixture = preflightFixture()
  try {
    fixture.pending()
    const result = fixture.run()
    expect(result.status, result.stderr).toBe(0)
    expect(result.stdout).toContain('native-build-reached')
  }
  finally {
    fixture.cleanup()
  }
}, 120_000)

it.each(['blocked', 'invalid-signature'])('stops release preflight for a %s service attestation', (failure) => {
  const fixture = preflightFixture()
  try {
    if (failure === 'blocked')
      fixture.resolution.state = 'blocked'
    else
      fixture.resolution.artifact.attestation.signature.value = Buffer.alloc(64).toString('base64url')
    fixture.saveResolution()
    const result = fixture.run()
    expect(result.status, result.stderr).not.toBe(0)
    expect(result.stdout).not.toContain('native-build-reached')
    if (failure === 'invalid-signature')
      expect(result.stderr).toContain('ATTESTATION_SIGNATURE_INVALID')
  }
  finally {
    fixture.cleanup()
  }
}, 120_000)

it('stops release preflight for an invalid root signature and accepts roots that both verify the service attestation', () => {
  const fixture = preflightFixture()
  try {
    for (const root of [fixture.primary, fixture.backup]) {
      const validSignature = root.keys[0]!.rootSignature
      root.keys[0]!.rootSignature = Buffer.alloc(64).toString('base64url')
      fixture.save(root)
      const invalid = fixture.run()
      expect(invalid.status, invalid.stderr).not.toBe(0)
      expect(invalid.stdout).not.toContain('native-build-reached')
      expect(invalid.stderr).toContain('TRUSTED_ROOT_SIGNATURE_INVALID')
      root.keys[0]!.rootSignature = validSignature
      fixture.save(root)
    }
    const valid = fixture.run()
    expect(valid.status, valid.stderr).toBe(0)
    expect(valid.stdout).toContain('native-build-reached')
  }
  finally {
    fixture.cleanup()
  }
}, 120_000)
