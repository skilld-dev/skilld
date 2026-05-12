/**
 * Local package reading: parsing dependency specifiers (link:/npm:/workspace:/etc.),
 * resolving installed versions from node_modules, and reading package.json.
 */

import type { LocalDependency, ResolvedPackage } from './types.ts'
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import { resolvePathSync } from 'mlly'
import { basename, dirname, join, resolve } from 'pathe'
import { readPackageJsonSafe } from '../core/package-json.ts'
import { fetchGitDocs, fetchReadme } from './github.ts'
import { normalizeRepoUrl, parseGitHubUrl } from './utils.ts'

export function parseVersionSpecifier(
  name: string,
  version: string,
  cwd: string,
): LocalDependency | null {
  if (version.startsWith('link:')) {
    const linkPath = resolve(cwd, version.slice(5))
    const linkedPkg = readPackageJsonSafe(join(linkPath, 'package.json'))
    if (linkedPkg) {
      return {
        name: (linkedPkg.parsed.name as string) || name,
        version: (linkedPkg.parsed.version as string) || '0.0.0',
      }
    }
    return null
  }

  if (version.startsWith('npm:')) {
    const specifier = version.slice(4)
    const atIndex = specifier.startsWith('@')
      ? specifier.indexOf('@', 1)
      : specifier.indexOf('@')
    const realName = atIndex > 0 ? specifier.slice(0, atIndex) : specifier
    return { name: realName, version: resolveInstalledVersion(realName, cwd) || '*' }
  }

  if (version.startsWith('file:') || version.startsWith('git:') || version.startsWith('git+')) {
    return null
  }

  const installed = resolveInstalledVersion(name, cwd)
  if (installed)
    return { name, version: installed }

  if (/^[\^~>=<\d]/.test(version))
    return { name, version: version.replace(/^[\^~>=<]+/, '') }

  if (version.startsWith('catalog:') || version.startsWith('workspace:'))
    return { name, version: '*' }

  return null
}

export function resolveInstalledVersion(name: string, cwd: string): string | null {
  try {
    const resolved = resolvePathSync(`${name}/package.json`, { url: cwd })
    return (readPackageJsonSafe(resolved)?.parsed.version as string) || null
  }
  catch {
    try {
      const entry = resolvePathSync(name, { url: cwd })
      let dir = dirname(entry)
      while (dir && basename(dir) !== 'node_modules') {
        const pkg = readPackageJsonSafe(join(dir, 'package.json'))
        if (pkg)
          return (pkg.parsed.version as string) || null
        const parent = dirname(dir)
        if (parent === dir)
          break
        dir = parent
      }
    }
    catch {}
    return null
  }
}

export async function readLocalDependencies(cwd: string): Promise<LocalDependency[]> {
  const pkgPath = join(cwd, 'package.json')
  const result = readPackageJsonSafe(pkgPath)
  if (!result) {
    throw new Error('No package.json found in current directory')
  }

  const pkg = result.parsed
  const deps: Record<string, string> = {
    ...pkg.dependencies as Record<string, string>,
    ...pkg.devDependencies as Record<string, string>,
  }

  const results: LocalDependency[] = []

  for (const [name, version] of Object.entries(deps)) {
    const parsed = parseVersionSpecifier(name, version, cwd)
    if (parsed) {
      results.push(parsed)
    }
  }

  return results
}

export interface LocalPackageInfo {
  name: string
  version: string
  description?: string
  repoUrl?: string
  localPath: string
}

export function readLocalPackageInfo(localPath: string): LocalPackageInfo | null {
  const result = readPackageJsonSafe(join(localPath, 'package.json'))
  if (!result)
    return null

  const pkg = result.parsed as Record<string, any>

  let repoUrl: string | undefined
  if (pkg.repository?.url) {
    repoUrl = normalizeRepoUrl(pkg.repository.url)
  }
  else if (typeof pkg.repository === 'string') {
    repoUrl = normalizeRepoUrl(pkg.repository)
  }

  return {
    name: pkg.name,
    version: pkg.version || '0.0.0',
    description: pkg.description,
    repoUrl,
    localPath,
  }
}

export async function resolveLocalPackageDocs(localPath: string): Promise<ResolvedPackage | null> {
  const info = readLocalPackageInfo(localPath)
  if (!info)
    return null

  const result: ResolvedPackage = {
    name: info.name,
    version: info.version,
    description: info.description,
    repoUrl: info.repoUrl,
  }

  if (info.repoUrl?.includes('github.com')) {
    const gh = parseGitHubUrl(info.repoUrl)
    if (gh) {
      const gitDocs = await fetchGitDocs(gh.owner, gh.repo, info.version, info.name)
      if (gitDocs) {
        result.gitDocsUrl = gitDocs.baseUrl
        result.gitRef = gitDocs.ref
        result.gitDocsFallback = gitDocs.fallback
      }

      const readmeUrl = await fetchReadme(gh.owner, gh.repo, undefined, result.gitRef)
      if (readmeUrl) {
        result.readmeUrl = readmeUrl
      }
    }
  }

  if (!result.readmeUrl && !result.gitDocsUrl) {
    const readmeFile = readdirSync(localPath).find(f => /^readme\.md$/i.test(f))
    if (readmeFile) {
      result.readmeUrl = pathToFileURL(join(localPath, readmeFile)).href
    }
  }

  if (!result.readmeUrl && !result.gitDocsUrl) {
    return null
  }

  return result
}

export function getInstalledSkillVersion(skillDir: string): string | null {
  const skillPath = join(skillDir, 'SKILL.md')
  if (!existsSync(skillPath))
    return null

  const content = readFileSync(skillPath, 'utf-8')
  const match = content.match(/^version:\s*"?([^"\n]+)"?/m)
  return match?.[1] || null
}
