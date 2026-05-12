/**
 * Unified entry for npm/crate package resolution.
 *
 * Owns the dispatch between `resolvePackageDocsWithAttempts` (npm),
 * `resolveCrateDocsWithAttempts` (crates.io), and the `link:` local
 * dependency fallback. Returns a single `PackageResolution` with all
 * the derived names callers need (identity, storage, lockfile, display).
 *
 * Used by every sync flow that takes a `package` or `crate:name` spec.
 */
import type { ResolveAttempt, ResolvedPackage } from './index.ts'
import { isCrateSpec, toCrateIdentity, toStoragePackageName } from '../core/prefix.ts'
import { resolveCrateDocsWithAttempts } from './crates.ts'
import { resolveLocalDep } from './local-dep.ts'
import { readLocalDependencies } from './local-package.ts'
import { resolvePackageDocsWithAttempts } from './resolver.ts'
import { parsePackageSpec } from './utils.ts'

const RESOLVE_STEP_LABELS: Record<string, string> = {
  'npm': 'npm registry',
  'github-docs': 'GitHub docs',
  'github-meta': 'GitHub meta',
  'github-search': 'GitHub search',
  'readme': 'README',
  'llms.txt': 'llms.txt',
  'crawl': 'website crawl',
  'local': 'node_modules',
}

export interface PackageResolution {
  /** Bare package name (lower-cased for crates). */
  packageName: string
  /** Public/lockfile name (`crate:` prefix retained). */
  identityPackageName: string
  /** Cache-safe name used for `~/.skilld/references/<name>@<version>/`. */
  storagePackageName: string
  isCrate: boolean
  /** Tag/version requested in the spec (e.g. "beta" from "vue@beta"). */
  requestedTag?: string
  /** Version pinned in the project's package.json, if any. */
  localVersion?: string
  /** Resolved package metadata, or null when no docs source matched. */
  resolved: ResolvedPackage | null
  /** Per-source resolution log (npm, github-docs, llms.txt, ...). */
  attempts: ResolveAttempt[]
  /** npm registry version even when docs resolution failed. */
  registryVersion?: string
}

export interface ResolvePackageOptions {
  cwd: string
  /**
   * Progress label callback. For npm, raw `ResolveStep` values are translated
   * to friendly names (`npm registry`, `GitHub docs`, ...) — the same set of
   * strings the legacy `RESOLVE_STEP_LABELS` map produced.
   */
  onProgress?: (message: string) => void
}

/**
 * Resolve `packageSpec` to a `PackageResolution`. `packageSpec` may be:
 *   - bare npm name: `vue`, `@scope/pkg`
 *   - npm with tag/version: `vue@beta`, `vue@3.4.0`
 *   - crate spec: `crate:tokio`, `crate:serde@1`
 *
 * Always returns a result; `resolved` is null when no docs source matched.
 * The caller decides whether to fall through to shipped-skills, npm
 * search-and-suggest, or hard error.
 */
export async function resolvePackageOrCrate(
  packageSpec: string,
  opts: ResolvePackageOptions,
): Promise<PackageResolution> {
  const { cwd, onProgress } = opts
  const isCrate = isCrateSpec(packageSpec)
  const normalizedSpec = isCrate ? packageSpec.slice('crate:'.length).trim() : packageSpec

  const { name: parsedName, tag: requestedTag } = parsePackageSpec(normalizedSpec)
  const packageName = isCrate ? parsedName.toLowerCase() : parsedName
  const identityPackageName = isCrate ? toCrateIdentity(packageName) : packageName
  const storagePackageName = toStoragePackageName(identityPackageName)

  const localDeps = isCrate ? [] : await readLocalDependencies(cwd).catch(() => [])
  const localVersion = isCrate ? undefined : localDeps.find(d => d.name === packageName)?.version

  const resolveResult = isCrate
    ? await resolveCrateDocsWithAttempts(packageName, {
        version: requestedTag,
        onProgress,
      })
    : await resolvePackageDocsWithAttempts(requestedTag ? normalizedSpec : packageName, {
        version: localVersion,
        cwd,
        onProgress: step => onProgress?.(RESOLVE_STEP_LABELS[step] ?? step),
      })

  let resolved = resolveResult.package
  if (!resolved && !isCrate) {
    onProgress?.(RESOLVE_STEP_LABELS.local!)
    resolved = await resolveLocalDep(packageName, cwd)
  }

  return {
    packageName,
    identityPackageName,
    storagePackageName,
    isCrate,
    requestedTag,
    localVersion,
    resolved,
    attempts: resolveResult.attempts,
    registryVersion: resolveResult.registryVersion,
  }
}
