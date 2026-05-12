/**
 * Issues step: fetch GitHub issues, write them to the repo-cache (or per-package
 * cache when the repo is unknown), and queue them for the search index.
 *
 * Skipped when cache already populated (existsSync guard).
 */

import type { StepResolver } from '../cascade.ts'
import type { TimelineCtx } from './types.ts'
import { existsSync } from 'node:fs'
import { writeToRepoCache } from '../../../cache/internal/storage.ts'
import { sanitizeMarkdown } from '../../../core/sanitize.ts'
import { fetchGitHubIssues, formatIssueAsMarkdown, generateIssueIndex, isGhAvailable } from '../../issues.ts'
import { defineStep } from '../cascade.ts'

export const issuesStep: StepResolver<TimelineCtx> = defineStep<TimelineCtx>({
  id: 'issues',
  canResolve: ctx => ctx.features.issues && !!ctx.repoInfo && isGhAvailable() && !existsSync(ctx.issuesDir),
  async run(ctx) {
    const { owner, repo } = ctx.repoInfo!
    ctx.onProgress('Fetching issues via GitHub API')
    const issues = await fetchGitHubIssues(owner, repo, 30, ctx.resolved.releasedAt, ctx.from).catch(() => [])
    if (issues.length === 0)
      return

    ctx.onProgress(`Caching ${issues.length} issues`)
    const docs = [
      ...issues.map(issue => ({
        path: `issues/issue-${issue.number}.md`,
        content: formatIssueAsMarkdown(issue),
      })),
      { path: 'issues/_INDEX.md', content: generateIssueIndex(issues) },
    ]
    writeToRepoCache(owner, repo, docs)

    for (const issue of issues) {
      ctx.docsToIndex.push({
        id: `issue-${issue.number}`,
        content: sanitizeMarkdown(`#${issue.number}: ${issue.title}\n\n${issue.body || ''}`),
        metadata: { package: ctx.packageName, source: `issues/issue-${issue.number}.md`, type: 'issue', number: issue.number },
      })
    }
  },
})
