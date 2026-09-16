import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { arch, platform, tmpdir } from 'node:os'
import { join } from 'node:path'
import { promisify } from 'node:util'

import { afterEach, it } from 'vitest'

import { buildReleaseManifest, generateReleaseKey, signReleaseManifest } from '../../scripts/release/release-signing.mjs'

const execFileAsync = promisify(execFile)
const temporaryDirectories = []
const supported = platform() === 'linux' && arch() === 'x64'

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { force: true, recursive: true })))
})

/**
 * Runs install.sh against a local release directory. Only `download` changes:
 * it copies from that directory instead of fetching over HTTPS.
 */
async function install({ tamperSignature = false, tamperBinary = false, publicKey } = {}) {
  const root = await mkdtemp(join(tmpdir(), 'skilld-install-'))
  temporaryDirectories.push(root)
  const release = join(root, 'release')
  await mkdir(release)
  const libc = (await execFileAsync('sh', ['-c', 'ls /lib/ld-musl-* >/dev/null 2>&1 && echo musl || echo gnu'])).stdout.trim()
  const asset = `skilld-cli-linux-x64-${libc}`
  const binary = '#!/bin/sh\necho "skilld 9.0.0"\n'
  const key = generateReleaseKey()
  const manifest = buildReleaseManifest('9.0.0', [{ name: asset, bytes: Buffer.from(binary) }])
  const signature = signReleaseManifest(tamperSignature ? manifest.replace('9.0.0', '9.0.1') : manifest, key.seed)
  await writeFile(join(release, 'skilld-release.txt'), manifest)
  await writeFile(join(release, 'skilld-release.sig'), `${signature}\n`)
  await writeFile(join(release, asset), tamperBinary ? `${binary}# changed\n` : binary)

  const script = (await readFile('install.sh', 'utf8'))
    .replace('__SKILLD_RELEASE_PUBLIC_KEY__', publicKey ?? key.publicKey)
    .replace(/download\(\) \{[\s\S]*?\n\}\n/, `download() {\n  cp "${release}/$(basename "$1")" "$2"\n}\n`)
  await writeFile(join(root, 'install.sh'), script)
  const installDir = join(root, 'bin')
  const result = await execFileAsync('sh', [join(root, 'install.sh')], {
    env: { ...process.env, SKILLD_INSTALL_DIR: installDir },
  }).then(output => ({ ok: true, ...output }), error => ({ ok: false, ...error }))
  return { result, installDir }
}

it.runIf(supported)('installs a signed release and writes the standalone marker', async () => {
  const { result, installDir } = await install()

  assert.ok(result.ok, result.stderr)
  assert.equal(result.stderr, '')
  assert.equal((await execFileAsync(join(installDir, 'skilld'), ['--version'])).stdout, 'skilld 9.0.0\n')
  assert.deepEqual(JSON.parse(await readFile(join(installDir, 'skilld-install.json'), 'utf8')), { channel: 'standalone' })
})

it.runIf(supported)('refuses a manifest signature that does not verify', async () => {
  const { result, installDir } = await install({ tamperSignature: true })

  assert.equal(result.ok, false)
  assert.match(result.stderr, /signature is invalid/)
  await assert.rejects(readFile(join(installDir, 'skilld')))
})

it.runIf(supported)('refuses a release signed by another key', async () => {
  const { result } = await install({ publicKey: generateReleaseKey().publicKey })

  assert.equal(result.ok, false)
  assert.match(result.stderr, /signature is invalid/)
})

it.runIf(supported)('refuses a binary that does not match its signed digest', async () => {
  const { result, installDir } = await install({ tamperBinary: true })

  assert.equal(result.ok, false)
  assert.match(result.stderr, /does not match its digest/)
  await assert.rejects(readFile(join(installDir, 'skilld')))
})
