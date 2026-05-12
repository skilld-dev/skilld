/**
 * Per-step unit tests for the content cascade.
 *
 * Each step is invoked directly with a fabricated `ContentCtx`, so we test the
 * step's own logic (canResolve gating, ctx mutations, fallback paths) without
 * running the full cascade.
 */

import type { ResolvedPackage } from '../../src/sources/types'
import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('../../src/sources/github', async () => {
  const actual = await vi.importActual<typeof import('../../src/sources/github')>('../../src/sources/github')
  return {
    ...actual,
    fetchGitDocs: vi.fn(),
    fetchReadmeContent: vi.fn(),
  }
})

vi.mock('../../src/sources/utils', () => ({
  fetchGitHubRaw: vi.fn(),
  fetchText: vi.fn(),
  verifyUrl: vi.fn(),
  $fetch: vi.fn(),
}))

vi.mock('../../src/sources/llms', () => ({
  fetchLlmsTxt: vi.fn(),
  downloadLlmsDocs: vi.fn(),
  normalizeLlmsLinks: (raw: string) => raw,
}))

vi.mock('../../src/sources/crawl', () => ({
  fetchCrawledDocs: vi.fn(),
  toCrawlPattern: (url: string) => `${url}/**`,
}))

const { fetchGitDocs, fetchReadmeContent } = await import('../../src/sources/github')
const { fetchGitHubRaw } = await import('../../src/sources/utils')
const { fetchLlmsTxt } = await import('../../src/sources/llms')
const { gitDocsStep } = await import('../../src/sources/resolvers/content/git-docs')
const { readmeStep } = await import('../../src/sources/resolvers/content/readme')
const { llmsTxtStep } = await import('../../src/sources/resolvers/content/llms-txt')

function makeCtx(resolved: Partial<ResolvedPackage> = {}) {
  return {
    packageName: 'pkg',
    resolved: { name: 'pkg', repoUrl: 'https://github.com/owner/repo', ...resolved } as ResolvedPackage,
    version: '1.0.0',
    onProgress: vi.fn(),
    isFrameworkDoc: () => true,
    docs: [] as Array<{ path: string, content: string }>,
    docsToIndex: [] as Array<{ id: string, content: string, metadata: Record<string, any> }>,
    warnings: [] as string[],
    docSource: 'readme',
    docsType: 'readme' as 'readme' | 'docs' | 'llms.txt',
  }
}

describe('readmeStep', () => {
  beforeEach(() => vi.resetAllMocks())

  it('skips when no readmeUrl', () => {
    const ctx = makeCtx({ readmeUrl: undefined })
    expect(readmeStep.canResolve?.(ctx)).toBe(false)
  })

  it('skips when docs already committed', () => {
    const ctx = makeCtx({ readmeUrl: 'https://x/README.md' })
    ctx.docs.push({ path: 'docs/foo.md', content: 'x' })
    expect(readmeStep.canResolve?.(ctx)).toBe(false)
  })

  it('writes README + index entry when fetch succeeds', async () => {
    vi.mocked(fetchReadmeContent).mockResolvedValue('# hello')
    const ctx = makeCtx({ readmeUrl: 'https://x/README.md' })
    await readmeStep.run(ctx)
    expect(ctx.docs).toEqual([{ path: 'docs/README.md', content: '# hello' }])
    expect(ctx.docsToIndex[0]?.metadata.type).toBe('doc')
  })

  it('writes nothing when fetch returns empty', async () => {
    vi.mocked(fetchReadmeContent).mockResolvedValue(null as unknown as string)
    const ctx = makeCtx({ readmeUrl: 'https://x/README.md' })
    await readmeStep.run(ctx)
    expect(ctx.docs).toHaveLength(0)
  })
})

describe('gitDocsStep — shallow fallback', () => {
  beforeEach(() => vi.resetAllMocks())

  it('writes git docs and sets docsType="docs" on success', async () => {
    vi.mocked(fetchGitDocs).mockResolvedValue({
      ref: 'v1.0.0',
      baseUrl: 'https://raw.githubusercontent.com/owner/repo/v1.0.0',
      files: ['docs/a.md', 'docs/b.md', 'docs/c.md', 'docs/d.md', 'docs/e.md', 'docs/f.md'],
      docsPrefix: '',
      fallback: false,
    })
    vi.mocked(fetchGitHubRaw).mockImplementation(async (url: string) => `content-${url.split('/').pop()}`)
    const ctx = makeCtx({ gitDocsUrl: 'https://github.com/owner/repo/tree/v1.0.0/docs' })
    await gitDocsStep.run(ctx)
    expect(ctx.docsType).toBe('docs')
    expect(ctx.docs.length).toBeGreaterThanOrEqual(6)
    expect(ctx.docSource).toContain('/tree/v1.0.0/docs')
  })

  it('commits nothing when result is shallow and llms.txt is available', async () => {
    vi.mocked(fetchGitDocs).mockResolvedValue({
      ref: 'v1.0.0',
      baseUrl: 'https://raw.githubusercontent.com/owner/repo/v1.0.0',
      files: ['docs/a.md', 'docs/b.md'], // 2 files — below MIN_GIT_DOCS
      docsPrefix: '',
      fallback: false,
    })
    vi.mocked(fetchGitHubRaw).mockImplementation(async (url: string) => `content-${url.split('/').pop()}`)
    const ctx = makeCtx({
      gitDocsUrl: 'https://github.com/owner/repo/tree/v1.0.0/docs',
      llmsUrl: 'https://x/llms.txt',
    })
    await gitDocsStep.run(ctx)
    expect(ctx.docs).toHaveLength(0)
    expect(ctx.docsType).toBe('readme') // unchanged from initial
  })

  it('still commits a small (but adequate) result when no llms.txt fallback exists', async () => {
    vi.mocked(fetchGitDocs).mockResolvedValue({
      ref: 'v1.0.0',
      baseUrl: 'https://raw.githubusercontent.com/owner/repo/v1.0.0',
      files: ['docs/a.md', 'docs/b.md'], // shallow, but no llmsUrl ⇒ commit anyway
      docsPrefix: '',
      fallback: false,
    })
    vi.mocked(fetchGitHubRaw).mockImplementation(async (url: string) => `content-${url.split('/').pop()}`)
    const ctx = makeCtx({
      gitDocsUrl: 'https://github.com/owner/repo/tree/v1.0.0/docs',
      llmsUrl: undefined,
    })
    await gitDocsStep.run(ctx)
    expect(ctx.docs).toHaveLength(2)
    expect(ctx.docsType).toBe('docs')
  })

  it('records a fallback warning when fetchGitDocs reports it', async () => {
    vi.mocked(fetchGitDocs).mockResolvedValue({
      ref: 'main',
      baseUrl: 'https://raw.githubusercontent.com/owner/repo/main',
      files: ['docs/a.md', 'docs/b.md', 'docs/c.md', 'docs/d.md', 'docs/e.md', 'docs/f.md'],
      docsPrefix: '',
      fallback: true,
    })
    vi.mocked(fetchGitHubRaw).mockImplementation(async (url: string) => `content-${url.split('/').pop()}`)
    const ctx = makeCtx({ gitDocsUrl: 'https://github.com/owner/repo/tree/main/docs' })
    await gitDocsStep.run(ctx)
    expect(ctx.warnings.some(w => w.includes('main'))).toBe(true)
  })
})

describe('llmsTxtStep', () => {
  beforeEach(() => vi.resetAllMocks())

  it('canResolve fires only when docs.length === 0 and llmsUrl present', () => {
    const yes = makeCtx({ llmsUrl: 'https://x/llms.txt' })
    expect(llmsTxtStep.canResolve?.(yes)).toBe(true)

    const occupied = makeCtx({ llmsUrl: 'https://x/llms.txt' })
    occupied.docs.push({ path: 'docs/a.md', content: 'x' })
    expect(llmsTxtStep.canResolve?.(occupied)).toBe(false)

    const noUrl = makeCtx({ llmsUrl: undefined })
    expect(llmsTxtStep.canResolve?.(noUrl)).toBe(false)
  })

  it('commits llms.txt as primary when no linked docs', async () => {
    vi.mocked(fetchLlmsTxt).mockResolvedValue({ raw: '# Docs', links: [] })
    const ctx = makeCtx({ llmsUrl: 'https://x/llms.txt' })
    await llmsTxtStep.run(ctx)
    expect(ctx.docs).toEqual([{ path: 'llms.txt', content: '# Docs' }])
    expect(ctx.docsType).toBe('llms.txt')
  })
})
