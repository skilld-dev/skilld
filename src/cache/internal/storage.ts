/**
 * Cache storage operations
 */

import type { CachedDoc, CachedPackage } from '../types.ts'
import { existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'pathe'
import { getRepoCacheDir, REFERENCES_DIR, REPOS_DIR, skillInternalDir } from '../../core/paths.ts'
import { resolvePkgDir } from '../../core/prepare.ts'
import { sanitizeMarkdown } from '../../core/sanitize.ts'
import { getCacheDir } from './version.ts'

/** Safely create a symlink, validating target is under REFERENCES_DIR or REPOS_DIR */
function safeSymlink(target: string, linkPath: string): void {
  const resolved = resolve(target)
  if (!resolved.startsWith(REFERENCES_DIR) && !resolved.startsWith(REPOS_DIR))
    throw new Error(`Symlink target outside allowed dirs: ${resolved}`)
  // Remove pre-existing symlink (check with lstat to detect symlinks)
  try {
    const stat = lstatSync(linkPath)
    if (stat.isSymbolicLink() || stat.isFile())
      unlinkSync(linkPath)
  }
  catch {}
  symlinkSync(target, linkPath, 'junction')
}

/**
 * Check if package is cached at given version
 * @internal
 */
export function isCached(name: string, version: string): boolean {
  return existsSync(getCacheDir(name, version))
}

/** Check if cache only has docs/README.md (pkg/ already has this) */
export function isReadmeOnlyCache(cacheDir: string): boolean {
  const docsDir = join(cacheDir, 'docs')
  if (!existsSync(docsDir))
    return false
  const files = readdirSync(docsDir)
  return files.length === 1 && files[0] === 'README.md'
}

export function inferDocsTypeFromCache(cacheDir: string, source?: string): 'llms.txt' | 'readme' | 'docs' {
  if (source?.includes('llms.txt') || existsSync(join(cacheDir, 'docs', 'llms.txt')))
    return 'llms.txt'
  return isReadmeOnlyCache(cacheDir) ? 'readme' : 'docs'
}

/**
 * Ensure cache directories exist
 */
export function ensureCacheDir(): void {
  mkdirSync(REFERENCES_DIR, { recursive: true, mode: 0o700 })
  mkdirSync(REPOS_DIR, { recursive: true, mode: 0o700 })
}

/**
 * Write docs to cache
 * @internal
 */
export function writeToCache(
  name: string,
  version: string,
  docs: CachedDoc[],
): string {
  const cacheDir = getCacheDir(name, version)
  mkdirSync(cacheDir, { recursive: true, mode: 0o700 })

  for (const doc of docs) {
    const filePath = join(cacheDir, doc.path)
    mkdirSync(join(filePath, '..'), { recursive: true, mode: 0o700 })
    writeFileSync(filePath, sanitizeMarkdown(doc.content), { mode: 0o600 })
  }

  return cacheDir
}

/**
 * Write docs to repo-level cache (~/.skilld/repos/<owner>/<repo>/)
 */
export function writeToRepoCache(
  owner: string,
  repo: string,
  docs: CachedDoc[],
): string {
  const repoDir = getRepoCacheDir(owner, repo)
  mkdirSync(repoDir, { recursive: true, mode: 0o700 })

  for (const doc of docs) {
    const filePath = join(repoDir, doc.path)
    mkdirSync(join(filePath, '..'), { recursive: true, mode: 0o700 })
    writeFileSync(filePath, sanitizeMarkdown(doc.content), { mode: 0o600 })
  }

  return repoDir
}

/**
 * Create symlink from .skilld dir to a repo-level cached subdirectory.
 *   .claude/skills/<skill>/.skilld/<subdir> -> ~/.skilld/repos/<owner>/<repo>/<subdir>
 */
export function linkRepoCachedDir(skillDir: string, owner: string, repo: string, subdir: string): void {
  const repoDir = getRepoCacheDir(owner, repo)
  const referencesDir = skillInternalDir(skillDir)
  const linkPath = join(referencesDir, subdir)
  const cachedPath = join(repoDir, subdir)

  mkdirSync(referencesDir, { recursive: true })

  if (existsSync(cachedPath)) {
    safeSymlink(cachedPath, linkPath)
  }
}

/**
 * Create symlink from .skilld dir to a cached subdirectory.
 * Unified handler for docs, issues, discussions, sections, releases.
 *
 * Structure:
 *   .claude/skills/<skill>/.skilld/<subdir> -> ~/.skilld/references/<pkg>@<version>/<subdir>
 *
 * The .skilld/ dirs are gitignored. After clone, `skilld install` recreates from lockfile.
 */
export function linkCachedDir(skillDir: string, name: string, version: string, subdir: string): void {
  const cacheDir = getCacheDir(name, version)
  const referencesDir = skillInternalDir(skillDir)
  const linkPath = join(referencesDir, subdir)
  const cachedPath = join(cacheDir, subdir)

  mkdirSync(referencesDir, { recursive: true })

  if (existsSync(cachedPath)) {
    safeSymlink(cachedPath, linkPath)
  }
}

/**
 * Create symlink from .skilld dir to package directory
 *
 * Structure:
 *   .claude/skills/<skill>/.skilld/pkg -> node_modules/<pkg> OR ~/.skilld/references/<pkg>@<version>/pkg
 *
 * This gives access to package.json, README.md, dist/, and any shipped docs/
 */
export function linkPkg(skillDir: string, name: string, cwd: string, version?: string): void {
  const pkgPath = resolvePkgDir(name, cwd, version)
  if (!pkgPath)
    return

  const referencesDir = skillInternalDir(skillDir)
  mkdirSync(referencesDir, { recursive: true })

  const pkgLinkPath = join(referencesDir, 'pkg')
  try {
    lstatSync(pkgLinkPath)
    unlinkSync(pkgLinkPath)
  }
  catch {}
  symlinkSync(pkgPath, pkgLinkPath, 'junction')
}

/**
 * Create named symlink from .skilld dir to package directory.
 * Short name = last segment of package name (e.g., @vue/reactivity → pkg-reactivity)
 *
 * Structure:
 *   .claude/skills/<skill>/.skilld/pkg-<short> -> node_modules/<pkg>
 */
export function linkPkgNamed(skillDir: string, name: string, cwd: string, version?: string): void {
  const pkgPath = resolvePkgDir(name, cwd, version)
  if (!pkgPath)
    return

  const shortName = name.split('/').pop()!.toLowerCase()
  const referencesDir = skillInternalDir(skillDir)
  mkdirSync(referencesDir, { recursive: true })

  const linkPath = join(referencesDir, `pkg-${shortName}`)
  try {
    lstatSync(linkPath)
    unlinkSync(linkPath)
  }
  catch {}
  symlinkSync(pkgPath, linkPath, 'junction')
}

/**
 * Write LLM-generated section outputs to global cache for cross-project reuse
 *
 * Structure:
 *   ~/.skilld/references/<pkg>@<version>/sections/_BEST_PRACTICES.md
 */
export function writeSections(name: string, version: string, sections: Array<{ file: string, content: string }>): void {
  const cacheDir = getCacheDir(name, version)
  const sectionsDir = join(cacheDir, 'sections')
  mkdirSync(sectionsDir, { recursive: true, mode: 0o700 })
  for (const { file, content } of sections) {
    writeFileSync(join(sectionsDir, file), content, { mode: 0o600 })
  }
}

/**
 * Read a cached section from the global references dir
 */
export function readCachedSection(name: string, version: string, file: string): string | null {
  const path = join(getCacheDir(name, version), 'sections', file)
  if (!existsSync(path))
    return null
  return readFileSync(path, 'utf-8')
}

/**
 * List all cached packages
 */
export function listCached(): CachedPackage[] {
  if (!existsSync(REFERENCES_DIR))
    return []

  return readdirSync(REFERENCES_DIR)
    .filter(name => name.includes('@'))
    .map((dir) => {
      const atIdx = dir.lastIndexOf('@')
      return { name: dir.slice(0, atIdx), version: dir.slice(atIdx + 1), dir: join(REFERENCES_DIR, dir) }
    })
}

/**
 * Read cached docs for a package
 */
export function readCachedDocs(name: string, version: string): CachedDoc[] {
  const cacheDir = getCacheDir(name, version)
  if (!existsSync(cacheDir))
    return []

  const docs: CachedDoc[] = []

  function walk(dir: string, prefix = '') {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const entryPath = join(dir, entry.name)
      const relativePath = prefix ? `${prefix}/${entry.name}` : entry.name

      if (entry.isDirectory()) {
        walk(entryPath, relativePath)
      }
      else if (entry.name.endsWith('.md') || entry.name.endsWith('.mdx')) {
        docs.push({
          path: relativePath,
          content: readFileSync(entryPath, 'utf-8'),
        })
      }
    }
  }

  walk(cacheDir)
  return docs
}

/**
 * Clear cache for a specific package
 */
export function clearCache(name: string, version: string): boolean {
  const cacheDir = getCacheDir(name, version)
  if (!existsSync(cacheDir))
    return false

  rmSync(cacheDir, { recursive: true })
  return true
}

/**
 * List files in .skilld directory (pkg + docs) as relative paths for prompt context
 * Returns paths like ./.skilld/pkg/README.md, ./.skilld/docs/api.md
 */
export function listReferenceFiles(skillDir: string, maxDepth = 3): string[] {
  const referencesDir = skillInternalDir(skillDir)
  if (!existsSync(referencesDir))
    return []

  const files: string[] = []

  function walk(dir: string, depth: number) {
    if (depth > maxDepth)
      return
    try {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const full = join(dir, entry.name)
        if (entry.isDirectory() || entry.isSymbolicLink()) {
          try {
            const stat = statSync(full)
            if (stat.isDirectory()) {
              walk(full, depth + 1)
              continue
            }
          }
          catch { continue }
        }
        if (entry.name.endsWith('.md')) {
          files.push(full)
        }
      }
    }
    catch {
      // Broken symlink or permission error
    }
  }

  walk(referencesDir, 0)
  return files
}
