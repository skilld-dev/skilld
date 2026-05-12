/**
 * Per-step unit tests for the timeline cascade.
 *
 * Steps share the canResolve invariants: a feature flag, repoInfo, gh CLI
 * availability, and an existsSync cache guard. The existsSync guard is the
 * load-bearing one (skip when cache already populated) and is asserted here.
 */

import type { TimelineCtx } from '../../src/sources/resolvers/timeline'
import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('node:fs', () => ({ existsSync: vi.fn() }))
vi.mock('../../src/sources/issues', () => ({
  isGhAvailable: vi.fn(),
  fetchGitHubIssues: vi.fn(),
  formatIssueAsMarkdown: (issue: { number: number, title: string }) => `# ${issue.title}`,
  generateIssueIndex: () => '# Index',
}))
vi.mock('../../src/sources/discussions', () => ({
  fetchGitHubDiscussions: vi.fn(),
  formatDiscussionAsMarkdown: (d: { title: string }) => `# ${d.title}`,
  generateDiscussionIndex: () => '# Index',
}))
vi.mock('../../src/cache/internal/storage', () => ({
  writeToRepoCache: vi.fn(),
  writeToCache: vi.fn(),
}))

const { existsSync } = await import('node:fs')
const { isGhAvailable, fetchGitHubIssues } = await import('../../src/sources/issues')
const { writeToRepoCache } = await import('../../src/cache/internal/storage')
const { issuesStep } = await import('../../src/sources/resolvers/timeline/issues')
const { discussionsStep } = await import('../../src/sources/resolvers/timeline/discussions')
const { releasesStep } = await import('../../src/sources/resolvers/timeline/releases')

function makeCtx(overrides: Partial<TimelineCtx> = {}): TimelineCtx {
  return {
    packageName: 'pkg',
    version: '1.0.0',
    resolved: { name: 'pkg', releasedAt: '2024-01-01', repoUrl: 'https://github.com/owner/repo' },
    features: { search: false, issues: true, discussions: true, releases: true },
    onProgress: vi.fn(),
    repoInfo: { owner: 'owner', repo: 'repo' },
    issuesDir: '/cache/owner/repo/issues',
    discussionsDir: '/cache/owner/repo/discussions',
    releasesPath: '/cache/owner/repo/releases',
    docsToIndex: [],
    ...overrides,
  }
}

describe('timeline canResolve invariants', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    vi.mocked(isGhAvailable).mockReturnValue(true)
    vi.mocked(existsSync).mockReturnValue(false)
  })

  for (const [name, step] of [['issues', issuesStep], ['discussions', discussionsStep], ['releases', releasesStep]] as const) {
    describe(name, () => {
      it('skips when feature flag is off', () => {
        const ctx = makeCtx({ features: { search: false, issues: false, discussions: false, releases: false } })
        expect(step.canResolve?.(ctx)).toBe(false)
      })

      it('skips when repoInfo is missing', () => {
        expect(step.canResolve?.(makeCtx({ repoInfo: undefined }))).toBe(false)
      })

      it('skips when gh CLI is unavailable', () => {
        vi.mocked(isGhAvailable).mockReturnValue(false)
        expect(step.canResolve?.(makeCtx())).toBe(false)
      })

      it('skips when cache dir already exists', () => {
        vi.mocked(existsSync).mockReturnValue(true)
        expect(step.canResolve?.(makeCtx())).toBe(false)
      })

      it('fires when all invariants pass', () => {
        expect(step.canResolve?.(makeCtx())).toBe(true)
      })
    })
  }
})

describe('issuesStep.run', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    vi.mocked(isGhAvailable).mockReturnValue(true)
    vi.mocked(existsSync).mockReturnValue(false)
  })

  it('writes issues to repo cache and queues index entries', async () => {
    vi.mocked(fetchGitHubIssues).mockResolvedValue([
      { number: 1, title: 'Bug X', body: 'broken' } as never,
      { number: 2, title: 'Feature Y', body: 'wanted' } as never,
    ])
    const ctx = makeCtx()
    await issuesStep.run(ctx)
    expect(writeToRepoCache).toHaveBeenCalledOnce()
    const call = vi.mocked(writeToRepoCache).mock.calls[0]!
    expect(call[0]).toBe('owner')
    expect(call[1]).toBe('repo')
    expect(call[2]).toHaveLength(3) // 2 issues + index
    expect(ctx.docsToIndex).toHaveLength(2)
    expect(ctx.docsToIndex[0]?.metadata.type).toBe('issue')
  })

  it('no-ops when fetch returns empty', async () => {
    vi.mocked(fetchGitHubIssues).mockResolvedValue([])
    const ctx = makeCtx()
    await issuesStep.run(ctx)
    expect(writeToRepoCache).not.toHaveBeenCalled()
    expect(ctx.docsToIndex).toHaveLength(0)
  })

  it('survives fetch errors without throwing', async () => {
    vi.mocked(fetchGitHubIssues).mockRejectedValue(new Error('rate limit'))
    const ctx = makeCtx()
    await expect(issuesStep.run(ctx)).resolves.toBeUndefined()
    expect(writeToRepoCache).not.toHaveBeenCalled()
  })
})
