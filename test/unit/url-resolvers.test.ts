/**
 * Per-step unit tests for the URL-resolution cascade.
 *
 * Symmetry parity with the content + timeline step tests: each resolver is
 * invoked with a fabricated `ResolveCtx`, asserting canResolve gating, ctx
 * mutations, and `ResolverOutcome` kinds (ok / skip / fatal).
 */

import type { ResolveCtx } from '../../src/sources/resolver-registry'
import type { NpmPackageInfo } from '../../src/sources/types'
import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('../../src/sources/npm-registry', () => ({
  fetchNpmPackage: vi.fn(),
  fetchNpmRegistryMeta: vi.fn(),
}))
vi.mock('../../src/sources/github', () => ({
  searchGitHubRepo: vi.fn(),
  fetchGitHubRepoMeta: vi.fn(),
  fetchGitDocs: vi.fn(),
}))
vi.mock('../../src/sources/package-registry', () => ({
  getCrawlUrl: vi.fn(),
}))

const { fetchNpmPackage, fetchNpmRegistryMeta } = await import('../../src/sources/npm-registry')
const { searchGitHubRepo, fetchGitHubRepoMeta, fetchGitDocs } = await import('../../src/sources/github')
const { getCrawlUrl } = await import('../../src/sources/package-registry')

const { npmResolver } = await import('../../src/sources/resolvers/npm')
const { githubSearchResolver } = await import('../../src/sources/resolvers/github-search')
const { githubMetaResolver } = await import('../../src/sources/resolvers/github-meta')
const { gitTagResolver } = await import('../../src/sources/resolvers/git-tag')
const { crawlUrlResolver } = await import('../../src/sources/resolvers/crawl-url')

function makeCtx(overrides: Partial<ResolveCtx> = {}): ResolveCtx {
  return {
    packageName: 'pkg',
    options: {},
    result: null,
    attempts: [],
    ...overrides,
  }
}

describe('crawlUrlResolver', () => {
  beforeEach(() => vi.resetAllMocks())

  it('skips when no result has been seeded', () => {
    expect(crawlUrlResolver.canResolve?.(makeCtx())).toBe(false)
  })

  it('fires once result exists', () => {
    expect(crawlUrlResolver.canResolve?.(makeCtx({ result: { name: 'pkg' } }))).toBe(true)
  })

  it('writes crawlUrl when registry has one', async () => {
    vi.mocked(getCrawlUrl).mockReturnValue('https://example.com/**')
    const ctx = makeCtx({ result: { name: 'pkg' } })
    await crawlUrlResolver.run(ctx)
    expect(ctx.result?.crawlUrl).toBe('https://example.com/**')
  })

  it('leaves crawlUrl undefined when registry has none', async () => {
    vi.mocked(getCrawlUrl).mockReturnValue(undefined)
    const ctx = makeCtx({ result: { name: 'pkg' } })
    await crawlUrlResolver.run(ctx)
    expect(ctx.result?.crawlUrl).toBeUndefined()
  })
})

describe('npmResolver — bootstrap', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    vi.mocked(fetchNpmRegistryMeta).mockResolvedValue({})
  })

  it('fatal-exits when package is missing from npm', async () => {
    vi.mocked(fetchNpmPackage).mockResolvedValue(null)
    const ctx = makeCtx()
    const outcome = await npmResolver.run(ctx)
    expect(outcome.kind).toBe('fatal')
    expect(ctx.attempts[0]?.status).toBe('not-found')
    expect(ctx.result).toBeNull()
  })

  it('seeds ctx.result with name/version/description/deps', async () => {
    vi.mocked(fetchNpmPackage).mockResolvedValue({
      name: 'pkg',
      version: '1.2.3',
      description: 'd',
      dependencies: { a: '^1' },
    } as NpmPackageInfo)
    vi.mocked(fetchNpmRegistryMeta).mockResolvedValue({ releasedAt: '2024-01-01', distTags: { latest: { version: '1.2.3' } } })
    const ctx = makeCtx()
    const outcome = await npmResolver.run(ctx)
    expect(outcome.kind).toBe('ok')
    expect(ctx.result).toMatchObject({
      name: 'pkg',
      version: '1.2.3',
      description: 'd',
      dependencies: { a: '^1' },
      releasedAt: '2024-01-01',
    })
  })

  it('normalizes object-form repository to https GitHub URL', async () => {
    vi.mocked(fetchNpmPackage).mockResolvedValue({
      name: 'pkg',
      version: '1.0.0',
      repository: { type: 'git', url: 'git+https://github.com/owner/repo.git', directory: 'packages/pkg' },
    } as NpmPackageInfo)
    const ctx = makeCtx()
    await npmResolver.run(ctx)
    expect(ctx.result?.repoUrl).toContain('github.com/owner/repo')
    expect(ctx.subdir).toBe('packages/pkg')
  })

  it('parses string-form `github:owner/repo` shorthand', async () => {
    vi.mocked(fetchNpmPackage).mockResolvedValue({
      name: 'pkg',
      version: '1.0.0',
      repository: 'github:owner/repo',
    } as NpmPackageInfo)
    const ctx = makeCtx()
    await npmResolver.run(ctx)
    expect(ctx.result?.repoUrl).toBe('https://github.com/owner/repo')
  })

  it('records docsUrl from homepage when not a useless URL', async () => {
    vi.mocked(fetchNpmPackage).mockResolvedValue({
      name: 'pkg',
      version: '1.0.0',
      homepage: 'https://docs.example.com',
    } as NpmPackageInfo)
    const ctx = makeCtx()
    await npmResolver.run(ctx)
    expect(ctx.result?.docsUrl).toBe('https://docs.example.com')
  })

  it('drops homepage that is just the GitHub repo URL', async () => {
    vi.mocked(fetchNpmPackage).mockResolvedValue({
      name: 'pkg',
      version: '1.0.0',
      homepage: 'https://github.com/owner/repo',
      repository: 'github:owner/repo',
    } as NpmPackageInfo)
    const ctx = makeCtx()
    await npmResolver.run(ctx)
    expect(ctx.result?.docsUrl).toBeUndefined()
  })
})

describe('githubSearchResolver — repo fallback', () => {
  beforeEach(() => vi.resetAllMocks())

  it('skips when result already has a repoUrl', () => {
    const ctx = makeCtx({ result: { name: 'pkg', repoUrl: 'https://github.com/x/y' } })
    expect(githubSearchResolver.canResolve?.(ctx)).toBe(false)
  })

  it('fires when result has no repoUrl', () => {
    const ctx = makeCtx({ result: { name: 'pkg' } })
    expect(githubSearchResolver.canResolve?.(ctx)).toBe(true)
  })

  it('writes discovered URL into result.repoUrl', async () => {
    vi.mocked(searchGitHubRepo).mockResolvedValue('https://github.com/owner/repo')
    const ctx = makeCtx({ result: { name: 'pkg' } })
    const outcome = await githubSearchResolver.run(ctx)
    expect(outcome.kind).toBe('ok')
    expect(ctx.result?.repoUrl).toBe('https://github.com/owner/repo')
  })

  it('returns skip when search finds nothing', async () => {
    vi.mocked(searchGitHubRepo).mockResolvedValue(null)
    const ctx = makeCtx({ result: { name: 'pkg' } })
    const outcome = await githubSearchResolver.run(ctx)
    expect(outcome.kind).toBe('skip')
    expect(ctx.result?.repoUrl).toBeUndefined()
  })
})

describe('githubMetaResolver — homepage from repo metadata', () => {
  beforeEach(() => vi.resetAllMocks())

  it('skips when no repoUrl', () => {
    expect(githubMetaResolver.canResolve?.(makeCtx({ result: { name: 'pkg' } }))).toBe(false)
  })

  it('skips when docsUrl is already set', () => {
    const ctx = makeCtx({ result: { name: 'pkg', repoUrl: 'https://github.com/x/y', docsUrl: 'https://docs' } })
    expect(githubMetaResolver.canResolve?.(ctx)).toBe(false)
  })

  it('writes homepage as docsUrl when found', async () => {
    vi.mocked(fetchGitHubRepoMeta).mockResolvedValue({ homepage: 'https://docs.example.com' } as never)
    const ctx = makeCtx({ result: { name: 'pkg', repoUrl: 'https://github.com/owner/repo' } })
    const outcome = await githubMetaResolver.run(ctx)
    expect(outcome.kind).toBe('ok')
    expect(ctx.result?.docsUrl).toBe('https://docs.example.com')
  })

  it('returns skip when no homepage in metadata', async () => {
    vi.mocked(fetchGitHubRepoMeta).mockResolvedValue({} as never)
    const ctx = makeCtx({ result: { name: 'pkg', repoUrl: 'https://github.com/owner/repo' } })
    const outcome = await githubMetaResolver.run(ctx)
    expect(outcome.kind).toBe('skip')
    expect(ctx.result?.docsUrl).toBeUndefined()
  })
})

describe('gitTagResolver — versioned docs', () => {
  beforeEach(() => vi.resetAllMocks())

  it('skips when result lacks a github repoUrl', () => {
    expect(gitTagResolver.canResolve?.(makeCtx({ result: { name: 'pkg' } }))).toBe(false)
    expect(gitTagResolver.canResolve?.(makeCtx({ result: { name: 'pkg', repoUrl: 'https://gitlab.com/x/y' } }))).toBe(false)
  })

  it('skips when no version is available from options or npm', async () => {
    const ctx = makeCtx({ result: { name: 'pkg', repoUrl: 'https://github.com/owner/repo' } })
    const outcome = await gitTagResolver.run(ctx)
    expect(outcome.kind).toBe('skip')
  })

  it('records gitDocsUrl + gitRef + fallback flag on success', async () => {
    vi.mocked(fetchGitDocs).mockResolvedValue({
      baseUrl: 'https://raw.githubusercontent.com/owner/repo/main',
      ref: 'main',
      files: ['docs/a.md'],
      docsPrefix: '',
      fallback: true,
      allFiles: ['docs/a.md', 'src/x.ts'],
    } as never)
    const ctx = makeCtx({
      result: { name: 'pkg', repoUrl: 'https://github.com/owner/repo' },
      options: { version: '1.2.3' },
    })
    const outcome = await gitTagResolver.run(ctx)
    expect(outcome.kind).toBe('ok')
    expect(ctx.result?.gitDocsUrl).toContain('raw.githubusercontent.com')
    expect(ctx.result?.gitRef).toBe('main')
    expect(ctx.result?.gitDocsFallback).toBe(true)
    expect(ctx.gitDocsAllFiles).toEqual(['docs/a.md', 'src/x.ts'])
  })

  it('prefers options.version over npm-supplied version', async () => {
    vi.mocked(fetchGitDocs).mockResolvedValue({
      baseUrl: 'x',
      ref: 'v9.9.9',
      files: ['docs/a.md'],
      docsPrefix: '',
      fallback: false,
      allFiles: [],
    } as never)
    const ctx = makeCtx({
      result: { name: 'pkg', repoUrl: 'https://github.com/owner/repo' },
      options: { version: '9.9.9' },
      npm: { name: 'pkg', version: '1.0.0' },
    })
    await gitTagResolver.run(ctx)
    expect(vi.mocked(fetchGitDocs).mock.calls[0]?.[2]).toBe('9.9.9')
  })

  it('returns skip when fetchGitDocs finds nothing', async () => {
    vi.mocked(fetchGitDocs).mockResolvedValue(null)
    const ctx = makeCtx({
      result: { name: 'pkg', repoUrl: 'https://github.com/owner/repo' },
      options: { version: '1.0.0' },
    })
    const outcome = await gitTagResolver.run(ctx)
    expect(outcome.kind).toBe('skip')
    expect(ctx.result?.gitDocsUrl).toBeUndefined()
  })
})
