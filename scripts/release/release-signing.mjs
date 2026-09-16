import { execFile } from 'node:child_process'
import { createHash, createPrivateKey, createPublicKey, generateKeyPairSync, sign, verify } from 'node:crypto'
import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'

import { nativePackageSpecs } from './native-packages.mjs'

const execFileAsync = promisify(execFile)
const RELEASE_DOMAIN = Buffer.from('skilld-release-v1\0')
const MANIFEST_HEADER = 'skilld-release-v1'
export const RELEASE_MANIFEST_ASSET = 'skilld-release.txt'
export const RELEASE_SIGNATURE_ASSET = 'skilld-release.sig'
/** PKCS#8 DER prefix for a raw 32-byte Ed25519 seed. */
const PKCS8_ED25519_PREFIX = Buffer.from('302e020100300506032b657004220420', 'hex')

export function releaseAssetName(spec) {
  return `skilld-${spec.directory}${spec.os === 'win32' ? '.exe' : ''}`
}

/** The exact bytes the release key signs. The CLI parses this format strictly. */
export function buildReleaseManifest(version, assets) {
  const lines = [...assets]
    .sort((left, right) => left.name.localeCompare(right.name))
    .map(asset => `${createHash('sha256').update(asset.bytes).digest('hex')}  ${asset.name}`)
  return `${MANIFEST_HEADER}\nversion ${version}\n${lines.join('\n')}\n`
}

function privateKey(seed) {
  const bytes = Buffer.from(seed, 'base64url')
  if (bytes.length !== 32 || bytes.toString('base64url') !== seed)
    throw new Error('SKILLD_RELEASE_SIGNING_KEY must be a canonical base64url 32-byte Ed25519 seed.')
  return createPrivateKey({ key: Buffer.concat([PKCS8_ED25519_PREFIX, bytes]), format: 'der', type: 'pkcs8' })
}

function signedMessage(manifest) {
  return Buffer.concat([RELEASE_DOMAIN, createHash('sha256').update(manifest).digest()])
}

export function releasePublicKey(seed) {
  return createPublicKey(privateKey(seed)).export({ format: 'jwk' }).x
}

export function signReleaseManifest(manifest, seed) {
  return sign(null, signedMessage(manifest), privateKey(seed)).toString('base64url')
}

export function verifyReleaseSignature(manifest, signature, publicKey) {
  const key = createPublicKey({ key: { kty: 'OKP', crv: 'Ed25519', x: publicKey }, format: 'jwk' })
  return verify(null, signedMessage(manifest), key, Buffer.from(signature, 'base64url'))
}

export function generateReleaseKey() {
  const { privateKey: key } = generateKeyPairSync('ed25519')
  const seed = key.export({ format: 'jwk' }).d
  return { seed, publicKey: releasePublicKey(seed) }
}

/** Copies every native binary to its release asset name, then writes the signed manifest. */
export async function stageRelease({ packagesRoot, outputRoot, version, seed, publicKey }) {
  if (releasePublicKey(seed) !== publicKey)
    throw new Error('SKILLD_RELEASE_SIGNING_KEY does not match SKILLD_RELEASE_PUBLIC_KEY.')
  await mkdir(outputRoot, { recursive: true })
  const assets = []
  for (const spec of nativePackageSpecs) {
    const name = releaseAssetName(spec)
    const source = join(packagesRoot, spec.directory, 'bin', spec.executable)
    await copyFile(source, join(outputRoot, name))
    assets.push({ name, bytes: await readFile(source) })
  }
  const manifest = buildReleaseManifest(version, assets)
  const signature = signReleaseManifest(manifest, seed)
  if (!verifyReleaseSignature(manifest, signature, publicKey))
    throw new Error('The release manifest signature does not verify.')
  await writeFile(join(outputRoot, RELEASE_MANIFEST_ASSET), manifest)
  await writeFile(join(outputRoot, RELEASE_SIGNATURE_ASSET), `${signature}\n`)
  return [...assets.map(asset => asset.name), RELEASE_MANIFEST_ASSET, RELEASE_SIGNATURE_ASSET]
}

async function setGithubSecret(name, value) {
  const child = execFileAsync('gh', ['secret', 'set', name])
  child.child.stdin.end(value)
  await child
}

async function main([command, ...rest]) {
  if (command === 'stage') {
    const [packagesRoot, outputRoot] = rest
    const manifest = JSON.parse(await readFile('package.json', 'utf8'))
    const names = await stageRelease({
      packagesRoot: resolve(packagesRoot),
      outputRoot: resolve(outputRoot),
      version: manifest.version,
      seed: process.env.SKILLD_RELEASE_SIGNING_KEY ?? '',
      publicKey: process.env.SKILLD_RELEASE_PUBLIC_KEY ?? '',
    })
    process.stdout.write(`${names.join('\n')}\n`)
    return
  }
  if (command === 'generate-key') {
    const { seed, publicKey } = generateReleaseKey()
    await setGithubSecret('SKILLD_RELEASE_SIGNING_KEY', seed)
    await execFileAsync('gh', ['variable', 'set', 'SKILLD_RELEASE_PUBLIC_KEY', '--body', publicKey])
    process.stdout.write(`Set SKILLD_RELEASE_SIGNING_KEY and SKILLD_RELEASE_PUBLIC_KEY.\nPublic key: ${publicKey}\n`)
    if (rest[0] === '--print-seed')
      process.stdout.write(`Back up this seed offline, then clear your terminal: ${seed}\n`)
    return
  }
  throw new Error('Expected stage or generate-key')
}

if (resolve(process.argv[1] ?? '') === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch((error) => {
    process.stderr.write(`${error.message}\n`)
    process.exitCode = 1
  })
}
