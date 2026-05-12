/**
 * NPM registry I/O — search, package metadata, dist-tags, tarball download.
 */

import type { NpmPackageInfo } from './types.ts'
import { spawnSync } from 'node:child_process'
import { createWriteStream, existsSync, mkdirSync, rmSync } from 'node:fs'
import { Writable } from 'node:stream'
import { join } from 'pathe'
import { getCacheDir } from '../cache/index.ts'
import { parsePackageSpec } from '../core/url.ts'
import { $fetch, SKILLD_USER_AGENT } from './utils.ts'

export async function searchNpmPackages(query: string, size = 5): Promise<Array<{ name: string, description?: string, version: string }>> {
  const data = await $fetch<{
    objects: Array<{ package: { name: string, description?: string, version: string } }>
  }>(`https://registry.npmjs.org/-/v1/search?text=${encodeURIComponent(query)}&size=${size}`).catch(() => null)

  if (!data?.objects?.length)
    return []

  return data.objects.map(o => ({
    name: o.package.name,
    description: o.package.description,
    version: o.package.version,
  }))
}

export async function fetchNpmPackage(packageName: string): Promise<NpmPackageInfo | null> {
  const data = await $fetch<NpmPackageInfo>(`https://unpkg.com/${packageName}/package.json`).catch(() => null)
  if (data)
    return data
  return $fetch<NpmPackageInfo>(`https://registry.npmjs.org/${packageName}/latest`).catch(() => null)
}

export interface DistTagInfo {
  version: string
  releasedAt?: string
}

export interface NpmRegistryMeta {
  releasedAt?: string
  distTags?: Record<string, DistTagInfo>
}

export async function fetchNpmRegistryMeta(packageName: string, version: string): Promise<NpmRegistryMeta> {
  const { name: barePackageName } = parsePackageSpec(packageName)
  const data = await $fetch<{
    'time'?: Record<string, string>
    'dist-tags'?: Record<string, string>
  }>(`https://registry.npmjs.org/${barePackageName}`, {
    headers: { Accept: 'application/vnd.npm.install-v1+json' },
  }).catch(() => null)

  if (!data)
    return {}

  const distTags: Record<string, DistTagInfo> | undefined = data['dist-tags']
    ? Object.fromEntries(
        Object.entries(data['dist-tags']).map(([tag, ver]) => [
          tag,
          { version: ver, releasedAt: data.time?.[ver] },
        ]),
      )
    : undefined

  return {
    releasedAt: data.time?.[version] || undefined,
    distTags,
  }
}

/**
 * Download and extract npm package tarball to cache directory.
 * Extracts to: ~/.skilld/references/<pkg>@<version>/pkg/
 */
export async function fetchPkgDist(name: string, version: string): Promise<string | null> {
  const cacheDir = getCacheDir(name, version)
  const pkgDir = join(cacheDir, 'pkg')

  if (existsSync(join(pkgDir, 'package.json')))
    return pkgDir

  const data = await $fetch<{ dist?: { tarball?: string } }>(
    `https://registry.npmjs.org/${name}/${version}`,
  ).catch(() => null)
  if (!data)
    return null
  const tarballUrl = data.dist?.tarball
  if (!tarballUrl)
    return null

  const tarballRes = await fetch(tarballUrl, {
    headers: { 'User-Agent': SKILLD_USER_AGENT },
  }).catch(() => null)

  if (!tarballRes?.ok || !tarballRes.body)
    return null

  mkdirSync(pkgDir, { recursive: true })

  const tmpTarball = join(cacheDir, '_pkg.tgz')
  const fileStream = createWriteStream(tmpTarball)
  const fileClosed = new Promise<void>(resolve => fileStream.once('close', resolve))

  const reader = tarballRes.body.getReader()

  try {
    await new Promise<void>((res, reject) => {
      const writable = new Writable({
        write(chunk, _encoding, callback) {
          fileStream.write(chunk, callback)
        },
      })
      writable.on('finish', () => {
        fileStream.end()
      })
      fileStream.on('close', () => res())
      writable.on('error', reject)
      fileStream.on('error', reject)

      function pump() {
        reader.read().then(({ done, value }) => {
          if (done) {
            writable.end()
            return
          }
          writable.write(value, () => pump())
        }).catch(reject)
      }
      pump()
    })

    const { status } = spawnSync('tar', ['xzf', tmpTarball, '--strip-components=1', '-C', pkgDir], { stdio: 'ignore' })
    if (status !== 0) {
      rmSync(pkgDir, { recursive: true, force: true })
      return null
    }

    return pkgDir
  }
  catch {
    rmSync(pkgDir, { recursive: true, force: true })
    return null
  }
  finally {
    reader.cancel().catch(() => {})
    fileStream.destroy()
    await fileClosed
    try {
      rmSync(tmpTarball, { force: true })
    }
    catch {}
  }
}

export async function fetchLatestVersion(packageName: string): Promise<string | null> {
  const data = await $fetch<{ version?: string }>(
    `https://unpkg.com/${packageName}/package.json`,
  ).catch(() => null)
  if (data?.version)
    return data.version

  const registry = await $fetch<{ 'dist-tags'?: Record<string, string> }>(
    `https://registry.npmjs.org/${packageName}`,
    { headers: { Accept: 'application/vnd.npm.install-v1+json' } },
  ).catch(() => null)
  return registry?.['dist-tags']?.latest || null
}
