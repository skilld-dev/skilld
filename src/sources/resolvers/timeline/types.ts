/**
 * Shared types for the timeline cascade (GitHub issues, discussions, releases).
 * Each step owns its own cache-write target (repo cache vs per-package cache).
 */

import type { FeaturesConfig } from '../../../core/config.ts'
import type { IndexDoc } from '../../content-resolver.ts'
import type { ResolvedPackage } from '../../types.ts'

export interface TimelineCtx {
  packageName: string
  version: string
  resolved: ResolvedPackage
  features: FeaturesConfig
  /** Lower-bound date for release/issue/discussion collection (ISO date). */
  from?: string
  onProgress: (message: string) => void
  /** When the repo is known on GitHub, timeline data caches per-repo. */
  repoInfo?: { owner: string, repo: string }
  /** Cache directories selected up-front based on repoInfo availability. */
  issuesDir: string
  discussionsDir: string
  releasesPath: string
  /** Accumulator: docs to feed the embedding index. */
  docsToIndex: IndexDoc[]
}
