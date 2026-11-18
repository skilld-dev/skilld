import { spawnSync } from 'node:child_process'
import { createHash, generateKeyPairSync, sign } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, it } from 'vitest'
import { parse } from 'yaml'

function rootFixture(rootKeyId: string) {
  const { publicKey, privateKey } = generateKeyPairSync('ed25519')
  const rootPublicKey = publicKey.export({ format: 'jwk' }).x!
  const key = {
    keyId: 'signer-1',
    algorithm: 'Ed25519',
    publicKey: rootPublicKey,
    notBefore: '2026-01-01T00:00:00.000Z',
    notAfter: '2027-01-01T00:00:00.000Z',
    status: 'active',
  }
  const statement = Buffer.from(JSON.stringify({ version: 1, rootKeyId, ...key }))
  const message = Buffer.concat([
    Buffer.from('skilld-trusted-key-v1\0'),
    createHash('sha256').update(statement).digest(),
  ])
  return {
    version: 1,
    rootKeyId,
    rootPublicKey,
    fetchedAt: '2026-11-18T00:00:00.000Z',
    keys: [{
      ...key,
      statement: statement.toString('base64url'),
      rootSignature: sign(null, message, privateKey).toString('base64url'),
    }],
  }
}

it('stops release preflight for an invalid signature on either pin and accepts valid roots', () => {
  const directory = mkdtempSync(join(tmpdir(), 'skilld-root-preflight-'))
  try {
    const workflow = parse(readFileSync('.github/workflows/release.yml', 'utf8'))
    const step = workflow.jobs['release-plan'].steps.find(
      (step: { name?: string }) => step.name === 'Require skilld.dev to serve both pinned roots',
    )
    const primary = rootFixture('primary-root')
    const backup = rootFixture('backup-root')
    const fixtures = [primary, backup]
    // Replace only the HTTP boundary. Run the real workflow gate and verifier.
    writeFileSync(join(directory, 'curl.ts'), `#!/usr/bin/env -S node --experimental-strip-types
import { copyFileSync } from 'node:fs'
import { join } from 'node:path'
const args = process.argv.slice(2)
const id = new URL(args.at(-1)!).pathname.split('/').at(-1)!
copyFileSync(join(process.env.ROOT_FIXTURES!, id + '.json'), args[args.indexOf('--output') + 1]!)
process.stdout.write('200')
`, { mode: 0o755 })
    symlinkSync('curl.ts', join(directory, 'curl'))
    const run = () => spawnSync('bash', ['-e', '-c', `${step.run}\nprintf 'native-build-reached'`], {
      encoding: 'utf8',
      env: {
        ...process.env,
        PATH: `${directory}:${process.env.PATH}`,
        ROOT_FIXTURES: directory,
        SKILLD_ROOT_KEY_ID: primary.rootKeyId,
        SKILLD_ROOT_PUBLIC_KEY: primary.rootPublicKey,
        SKILLD_BACKUP_ROOT_KEY_ID: backup.rootKeyId,
        SKILLD_BACKUP_ROOT_PUBLIC_KEY: backup.rootPublicKey,
      },
    })
    for (const fixture of fixtures)
      writeFileSync(join(directory, `${fixture.rootKeyId}.json`), JSON.stringify(fixture))

    for (const fixture of fixtures) {
      const validSignature = fixture.keys[0]!.rootSignature
      fixture.keys[0]!.rootSignature = Buffer.alloc(64).toString('base64url')
      writeFileSync(join(directory, `${fixture.rootKeyId}.json`), JSON.stringify(fixture))
      const invalid = run()
      expect(invalid.status, invalid.stderr).not.toBe(0)
      expect(invalid.stdout).not.toContain('native-build-reached')
      expect(invalid.stderr).toContain('TRUSTED_ROOT_SIGNATURE_INVALID')
      fixture.keys[0]!.rootSignature = validSignature
      writeFileSync(join(directory, `${fixture.rootKeyId}.json`), JSON.stringify(fixture))
    }
    const valid = run()
    expect(valid.status, valid.stderr).toBe(0)
    expect(valid.stdout).toContain('native-build-reached')
  }
  finally {
    rmSync(directory, { recursive: true, force: true })
  }
}, 120_000)
