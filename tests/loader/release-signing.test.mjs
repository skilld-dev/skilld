import assert from 'node:assert/strict'
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, it } from 'vitest'

import { nativePackageSpecs } from '../../scripts/release/native-packages.mjs'
import {
  buildReleaseManifest,
  generateReleaseKey,
  releaseAssetName,
  signReleaseManifest,
  stageRelease,
  verifyReleaseSignature,
} from '../../scripts/release/release-signing.mjs'

const temporaryDirectories = []

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { force: true, recursive: true })))
})

it('stages every native binary under its asset name with a signed manifest', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skilld-release-'))
  temporaryDirectories.push(root)
  for (const spec of nativePackageSpecs) {
    const bin = join(root, 'packages', spec.directory, 'bin')
    await mkdir(bin, { recursive: true })
    await writeFile(join(bin, spec.executable), `binary for ${spec.directory}`)
    await chmod(join(bin, spec.executable), 0o755)
  }
  const { seed, publicKey } = generateReleaseKey()

  const names = await stageRelease({
    packagesRoot: join(root, 'packages'),
    outputRoot: join(root, 'assets'),
    version: '3.1.0',
    seed,
    publicKey,
  })

  assert.equal(names.length, nativePackageSpecs.length + 2)
  assert.ok(names.includes('skilld-cli-win32-x64-msvc.exe'))
  const manifest = await readFile(join(root, 'assets', 'skilld-release.txt'), 'utf8')
  const signature = (await readFile(join(root, 'assets', 'skilld-release.sig'), 'utf8')).trim()
  assert.ok(manifest.startsWith('skilld-release-v1\nversion 3.1.0\n'))
  assert.equal(manifest.trim().split('\n').length, nativePackageSpecs.length + 2)
  assert.ok(verifyReleaseSignature(manifest, signature, publicKey))
  assert.equal(
    await readFile(join(root, 'assets', releaseAssetName(nativePackageSpecs[0])), 'utf8'),
    `binary for ${nativePackageSpecs[0].directory}`,
  )
})

it('refuses to stage a release when the signing key does not match the pinned public key', async () => {
  const signer = generateReleaseKey()
  const other = generateReleaseKey()

  await assert.rejects(
    stageRelease({ packagesRoot: '/nonexistent', outputRoot: '/nonexistent', version: '3.1.0', seed: signer.seed, publicKey: other.publicKey }),
    /does not match SKILLD_RELEASE_PUBLIC_KEY/,
  )
})

it('rejects a changed manifest', () => {
  const { seed, publicKey } = generateReleaseKey()
  const manifest = buildReleaseManifest('3.1.0', [{ name: 'skilld-cli-darwin-arm64', bytes: Buffer.from('a') }])
  const signature = signReleaseManifest(manifest, seed)

  assert.ok(verifyReleaseSignature(manifest, signature, publicKey))
  assert.equal(verifyReleaseSignature(manifest.replace('3.1.0', '3.2.0'), signature, publicKey), false)
})
