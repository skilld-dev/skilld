import { readFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { setTimeout } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'
import { nativePackageSpecs, npmTagForVersion, verifyReleaseVersions } from './native-packages.mjs'

export interface NpmPackage {
  name: string
  version: string
  tag: string
}

interface WaitOptions {
  packages: readonly NpmPackage[]
  fetch?: typeof fetch
  now?: () => number
  sleep?: (ms: number) => Promise<void>
  onProgress?: (message: string) => void
  timeoutMs?: number
  intervalMs?: number
}

type Probe = { _tag: 'Available' } | { _tag: 'Pending', reason: string }

/** npm scans uploads before making versions and tags available to consumers. */
export async function waitForNpmPackages(options: WaitOptions): Promise<void> {
  const { packages, onProgress } = options
  const fetchRegistry = options.fetch ?? globalThis.fetch
  const now = options.now ?? Date.now
  const sleep = options.sleep ?? (async (ms) => {
    await setTimeout(ms)
  })
  const timeoutMs = options.timeoutMs ?? 15 * 60_000
  const intervalMs = options.intervalMs ?? 30_000
  if (timeoutMs <= 0 || intervalMs <= 0 || !Number.isFinite(timeoutMs) || !Number.isFinite(intervalMs))
    throw new Error('npm availability requires positive, finite wait times.')
  const deadline = now() + timeoutMs
  let pending = [...packages]
  while (now() < deadline) {
    const results = await Promise.all(pending.map(async (pkg) => {
      for (const selector of [pkg.version, pkg.tag]) {
        if (now() >= deadline)
          return pkg
        const result = await probe(pkg, selector, fetchRegistry, Math.max(1, Math.min(10_000, deadline - now())))
        if (now() >= deadline)
          return pkg
        if (result._tag === 'Pending') {
          onProgress?.(`${pkg.name}@${pkg.version}: ${result.reason}`)
          return pkg
        }
      }
      return null
    }))
    pending = results.filter((pkg): pkg is NpmPackage => pkg !== null)
    if (pending.length === 0) {
      onProgress?.('All release packages are available from npm.')
      return
    }
    const remaining = deadline - now()
    if (remaining > 0)
      await sleep(Math.min(intervalMs, remaining))
  }
  throw new Error(`npm availability timed out: ${pending.map(pkg => `${pkg.name}@${pkg.version}`).join(', ')}`)
}

async function probe(pkg: NpmPackage, selector: string, fetchRegistry: typeof fetch, timeoutMs: number): Promise<Probe> {
  const url = `https://registry.npmjs.org/${encodeURIComponent(pkg.name)}/${encodeURIComponent(selector)}`
  const fetched = await fetchRegistry(url, { signal: AbortSignal.timeout(timeoutMs), redirect: 'error', headers: { accept: 'application/json' } })
    .then(response => ({ _tag: 'Response', response }) as const, error => ({ _tag: 'Pending', reason: `Registry request failed: ${String(error)}` }) as const)
  if (fetched._tag === 'Pending')
    return fetched
  const { response } = fetched
  if (response.status === 404 || response.status === 429 || response.status >= 500)
    return { _tag: 'Pending', reason: `${selector}: HTTP ${response.status}` }
  if (response.status !== 200)
    throw new Error(`${pkg.name}: npm registry returned HTTP ${response.status}.`)
  if (Number(response.headers.get('content-length')) > 1024 * 1024)
    throw new Error(`${pkg.name}: npm metadata exceeds its byte limit.`)
  const read = await response.text().then(text => ({ _tag: 'Text', text }) as const, error => ({ _tag: 'Pending', reason: `Registry body failed: ${String(error)}` }) as const)
  if (read._tag === 'Pending')
    return read
  if (Buffer.byteLength(read.text) > 1024 * 1024)
    throw new Error(`${pkg.name}: npm metadata exceeds its byte limit.`)
  const body: unknown = JSON.parse(read.text)
  if (typeof body !== 'object' || body === null || !('name' in body) || body.name !== pkg.name)
    throw new Error(`${pkg.name}: npm returned a different package.`)
  if (!('version' in body) || typeof body.version !== 'string')
    throw new Error(`${pkg.name}: npm returned invalid version metadata.`)
  if (body.version === pkg.version)
    return { _tag: 'Available' }
  if (selector === pkg.version)
    throw new Error(`${pkg.name}: npm returned a different package version.`)
  return { _tag: 'Pending', reason: `${selector} still points to ${body.version}.` }
}

async function main(): Promise<void> {
  const root = process.cwd()
  const tag = process.argv[2]
  await verifyReleaseVersions(root, tag)
  const directories = [root, join(root, 'packages/harness'), ...nativePackageSpecs.map(spec => join(root, 'packages', spec.directory))]
  const packages = await Promise.all(directories.map(async (directory): Promise<NpmPackage> => {
    const manifest: unknown = JSON.parse(await readFile(join(directory, 'package.json'), 'utf8'))
    if (typeof manifest !== 'object' || manifest === null || !('name' in manifest) || typeof manifest.name !== 'string' || !('version' in manifest) || typeof manifest.version !== 'string')
      throw new Error(`Invalid release manifest: ${directory}`)
    return { name: manifest.name, version: manifest.version, tag: npmTagForVersion(manifest.version) }
  }))
  await waitForNpmPackages({ packages, onProgress: console.log })
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error: unknown) => {
    console.error(error instanceof Error ? error.message : String(error))
    process.exitCode = 1
  })
}
