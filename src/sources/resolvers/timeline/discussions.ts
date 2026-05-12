/**
 * Discussions step: fetch GitHub discussions, write to repo cache,
 * queue for the search index.
 */

import type { StepResolver } from '../cascade.ts'
import type { TimelineCtx } from './types.ts'
import { existsSync } from 'node:fs'
import { writeToRepoCache } from '../../../cache/internal/storage.ts'
import { sanitizeMarkdown } from '../../../core/sanitize.ts'
import { fetchGitHubDiscussions, formatDiscussionAsMarkdown, generateDiscussionIndex } from '../../discussions.ts'
import { isGhAvailable } from '../../issues.ts'
import { defineStep } from '../cascade.ts'

export const discussionsStep: StepResolver<TimelineCtx> = defineStep<TimelineCtx>({
  id: 'discussions',
  canResolve: ctx => ctx.features.discussions && !!ctx.repoInfo && isGhAvailable() && !existsSync(ctx.discussionsDir),
  async run(ctx) {
    const { owner, repo } = ctx.repoInfo!
    ctx.onProgress('Fetching discussions via GitHub API')
    const discussions = await fetchGitHubDiscussions(owner, repo, 20, ctx.resolved.releasedAt, ctx.from).catch(() => [])
    if (discussions.length === 0)
      return

    ctx.onProgress(`Caching ${discussions.length} discussions`)
    const docs = [
      ...discussions.map(d => ({
        path: `discussions/discussion-${d.number}.md`,
        content: formatDiscussionAsMarkdown(d),
      })),
      { path: 'discussions/_INDEX.md', content: generateDiscussionIndex(discussions) },
    ]
    writeToRepoCache(owner, repo, docs)

    for (const d of discussions) {
      ctx.docsToIndex.push({
        id: `discussion-${d.number}`,
        content: sanitizeMarkdown(`#${d.number}: ${d.title}\n\n${d.body || ''}`),
        metadata: { package: ctx.packageName, source: `discussions/discussion-${d.number}.md`, type: 'discussion', number: d.number },
      })
    }
  },
})
