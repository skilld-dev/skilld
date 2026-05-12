/**
 * Per-repository cache facade. Closes over `(owner, repo)` so call sites stop
 * passing the tuple through chained primitives. Used for issues/discussions/
 * releases pulled from a GitHub repo that may map to multiple npm packages.
 */

import type { CachedDoc } from './types.ts'
import { getRepoCacheDir } from '../core/paths.ts'
import { linkRepoCachedDir, writeToRepoCache } from './internal/storage.ts'

export interface RepoCache {
  readonly owner: string
  readonly repo: string
  /** Resolved cache directory under `~/.skilld/repos/<owner>/<repo>/` */
  readonly dir: string
  write: (docs: CachedDoc[]) => void
  linkInto: (skillDir: string, subdir: string) => void
}

export function createRepoCache(owner: string, repo: string): RepoCache {
  return {
    owner,
    repo,
    get dir() { return getRepoCacheDir(owner, repo) },
    write: docs => void writeToRepoCache(owner, repo, docs),
    linkInto: (skillDir, subdir) => linkRepoCachedDir(skillDir, owner, repo, subdir),
  }
}
