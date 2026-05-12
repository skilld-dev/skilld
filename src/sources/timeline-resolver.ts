/**
 * Timeline-references cascade: GitHub issues, discussions, and releases.
 *
 * Each step is a `StepResolver<TimelineCtx>` in `./resolvers/timeline/`,
 * with its own `existsSync` cache guard and feature-flag gate. Repo-level
 * data lives at `~/.skilld/references/<owner>/<repo>/{issues,discussions,releases}/`
 * when a GitHub repo is known; otherwise the cascade skips (no per-package
 * fallback — that path was unreachable since each step needs owner/repo to
 * call the GitHub API).
 */

import type { FeaturesConfig } from '../core/config.ts'
import type { IndexDoc } from './content-resolver.ts'
import type { TimelineCtx } from './resolvers/timeline/index.ts'
import type { ResolvedPackage } from './types.ts'
import { existsSync } from 'node:fs'
import { join } from 'pathe'
import { getCacheDir, getRepoCacheDir } from '../cache/index.ts'
import { parseGitHubUrl } from '../core/url.ts'
import { walkSteps } from './resolvers/cascade.ts'
import { defaultTimelineSteps } from './resolvers/timeline/index.ts'

export interface TimelineReferences {
  docsToIndex: IndexDoc[]
  hasIssues: boolean
  hasDiscussions: boolean
  hasReleases: boolean
  repoInfo?: { owner: string, repo: string }
}

export interface ResolveTimelineOptions {
  packageName: string
  resolved: ResolvedPackage
  version: string
  features: FeaturesConfig
  /** Lower-bound date for release/issue/discussion collection (ISO date) */
  from?: string
  onProgress: (message: string) => void
}

export async function resolveTimelineReferences(opts: ResolveTimelineOptions): Promise<TimelineReferences> {
  const { packageName, resolved, version, features, from, onProgress } = opts

  const gh = resolved.repoUrl ? parseGitHubUrl(resolved.repoUrl) : null
  const repoInfo = gh ? { owner: gh.owner, repo: gh.repo } : undefined
  const repoCacheDir = repoInfo ? getRepoCacheDir(repoInfo.owner, repoInfo.repo) : null
  const cacheDir = getCacheDir(packageName, version)

  const ctx: TimelineCtx = {
    packageName,
    version,
    resolved,
    features,
    from,
    onProgress,
    repoInfo,
    issuesDir: repoCacheDir ? join(repoCacheDir, 'issues') : join(cacheDir, 'issues'),
    discussionsDir: repoCacheDir ? join(repoCacheDir, 'discussions') : join(cacheDir, 'discussions'),
    releasesPath: repoCacheDir ? join(repoCacheDir, 'releases') : join(cacheDir, 'releases'),
    docsToIndex: [],
  }

  await walkSteps(defaultTimelineSteps, ctx)

  return {
    docsToIndex: ctx.docsToIndex,
    hasIssues: features.issues && existsSync(ctx.issuesDir),
    hasDiscussions: features.discussions && existsSync(ctx.discussionsDir),
    hasReleases: features.releases && existsSync(ctx.releasesPath),
    repoInfo,
  }
}
