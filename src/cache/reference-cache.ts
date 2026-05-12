/**
 * Per-package reference cache facade. Closes over `(packageName, version)` so
 * call sites stop threading those tuples through chained primitives.
 */

import type { FeaturesConfig } from '../core/config.ts'
import type { CachedDoc, CachedReferencesResult, LoadCachedReferencesOptions } from './types.ts'
import { getPkgKeyFiles, hasShippedDocs, resolvePkgDir } from '../core/prepare.ts'
import {
  clearSkillInternalDir,
  detectDocsType,
  ejectReferences,
  forceClearCache,
  linkAllReferences,
  loadCachedReferences,
} from './internal/references.ts'
import {
  ensureCacheDir,
  inferDocsTypeFromCache,
  isCached,
  isReadmeOnlyCache,
  linkPkgNamed,
  readCachedDocs,
  readCachedSection,
  writeSections,
  writeToCache,
} from './internal/storage.ts'
import { getCacheDir, getVersionKey } from './internal/version.ts'

export interface ReferenceCacheLinkOpts {
  extraPackages?: Array<{ name: string, version?: string }>
  features?: FeaturesConfig
  repoInfo?: { owner: string, repo: string }
}

export interface ReferenceCacheEjectOpts {
  features?: FeaturesConfig
  repoInfo?: { owner: string, repo: string }
}

export interface ReferenceCache {
  readonly packageName: string
  /** May be undefined when caller only knows the package name (e.g. merge against a lock without a recorded version). Cache-keyed methods/getters throw in that case; pkg/has/keyFiles fall back to node_modules. */
  readonly version: string | undefined
  /** Throws if accessed when version is undefined. */
  readonly versionKey: string
  /** Throws if accessed when version is undefined. */
  readonly dir: string
  ensure: () => void
  has: () => boolean
  isReadmeOnly: () => boolean
  inferDocsType: (source?: string) => 'llms.txt' | 'readme' | 'docs'
  write: (docs: CachedDoc[]) => void
  writeSections: (sections: Array<{ file: string, content: string }>) => void
  readSection: (file: string) => string | null
  readDocs: () => CachedDoc[]
  detectDocs: (
    repoUrl?: string,
    llmsUrl?: string,
  ) => { docsType: 'docs' | 'llms.txt' | 'readme', docSource?: string }
  load: (
    opts: Omit<LoadCachedReferencesOptions, 'packageName' | 'version'>,
  ) => CachedReferencesResult
  linkInto: (
    skillDir: string,
    cwd: string,
    docsType: string,
    opts?: ReferenceCacheLinkOpts,
  ) => void
  linkPkgNamed: (skillDir: string, cwd: string) => void
  eject: (
    skillDir: string,
    cwd: string,
    docsType: string,
    opts?: ReferenceCacheEjectOpts,
  ) => void
  clearForce: (repoInfo?: { owner: string, repo: string }) => void
  clearSkillInternal: (skillDir: string) => void
  pkgDir: (cwd: string) => string | null
  hasShipped: (cwd: string) => boolean
  keyFiles: (cwd: string) => string[]
}

export function createReferenceCache(packageName: string, version?: string): ReferenceCache {
  const requireVersion = (op: string): string => {
    if (!version)
      throw new Error(`ReferenceCache.${op} requires a version (package: ${packageName})`)
    return version
  }
  return {
    packageName,
    version,
    get versionKey() { return getVersionKey(requireVersion('versionKey')) },
    get dir() { return getCacheDir(packageName, requireVersion('dir')) },
    ensure: () => ensureCacheDir(),
    has: () => !!version && isCached(packageName, version),
    isReadmeOnly: () => isReadmeOnlyCache(getCacheDir(packageName, requireVersion('isReadmeOnly'))),
    inferDocsType: source => inferDocsTypeFromCache(getCacheDir(packageName, requireVersion('inferDocsType')), source),
    write: docs => void writeToCache(packageName, requireVersion('write'), docs),
    writeSections: sections => writeSections(packageName, requireVersion('writeSections'), sections),
    readSection: file => readCachedSection(packageName, requireVersion('readSection'), file),
    readDocs: () => readCachedDocs(packageName, requireVersion('readDocs')),
    detectDocs: (repoUrl, llmsUrl) =>
      detectDocsType(packageName, requireVersion('detectDocs'), repoUrl, llmsUrl),
    load: opts =>
      loadCachedReferences({ ...opts, packageName, version: requireVersion('load') }),
    linkInto: (skillDir, cwd, docsType, opts) =>
      linkAllReferences(
        skillDir,
        packageName,
        cwd,
        requireVersion('linkInto'),
        docsType,
        opts?.extraPackages,
        opts?.features,
        opts?.repoInfo,
      ),
    linkPkgNamed: (skillDir, cwd) =>
      linkPkgNamed(skillDir, packageName, cwd, version),
    eject: (skillDir, cwd, docsType, opts) =>
      ejectReferences(
        skillDir,
        packageName,
        cwd,
        requireVersion('eject'),
        docsType,
        opts?.features,
        opts?.repoInfo,
      ),
    clearForce: repoInfo => forceClearCache(packageName, requireVersion('clearForce'), repoInfo),
    clearSkillInternal: skillDir => clearSkillInternalDir(skillDir),
    pkgDir: cwd => resolvePkgDir(packageName, cwd, version),
    hasShipped: cwd => hasShippedDocs(packageName, cwd, version),
    keyFiles: cwd => getPkgKeyFiles(packageName, cwd, version),
  }
}
