import { beforeEach, describe, expect, it, vi } from 'vitest'

// Mock fs
vi.mock('node:fs', async () => {
  const actual = await vi.importActual<typeof import('node:fs')>('node:fs')
  return {
    ...actual,
    existsSync: vi.fn(),
    readdirSync: vi.fn(),
    readFileSync: vi.fn(),
    mkdirSync: vi.fn(),
    rmSync: vi.fn(),
    copyFileSync: vi.fn(),
  }
})

// Shared mocks so callers that import internals (e.g. sources/timeline-resolver,
// cache/internal/references) hit fakes instead of touching disk.
const sharedWriteToCache = vi.fn()
const sharedWriteToRepoCache = vi.fn()
const sharedClearCache = vi.fn()
const sharedLinkRepoCachedDir = vi.fn()
const sharedLinkCachedDir = vi.fn()
const sharedLinkPkg = vi.fn()
const sharedLinkPkgNamed = vi.fn()

vi.mock('../../src/cache/internal/storage', () => ({
  writeToCache: sharedWriteToCache,
  writeToRepoCache: sharedWriteToRepoCache,
  clearCache: sharedClearCache,
  linkRepoCachedDir: sharedLinkRepoCachedDir,
  linkCachedDir: sharedLinkCachedDir,
  linkPkg: sharedLinkPkg,
  linkPkgNamed: sharedLinkPkgNamed,
  ensureCacheDir: vi.fn(),
  inferDocsTypeFromCache: vi.fn(() => 'readme'),
  isCached: vi.fn(() => false),
  isReadmeOnlyCache: vi.fn(() => false),
  readCachedDocs: vi.fn(() => []),
  readCachedSection: vi.fn(() => null),
  writeSections: vi.fn(),
  listCached: vi.fn(() => []),
  listReferenceFiles: vi.fn(() => []),
}))

vi.mock('../../src/cache/internal/version', () => ({
  getCacheDir: vi.fn(() => '/mock-cache/references/test-pkg@1.0.0'),
  getCacheKey: vi.fn((name: string, version: string) => `${name}@${version}`),
  getVersionKey: vi.fn((v: string) => v),
}))

vi.mock('../../src/cache', () => {
  const writeToCache = sharedWriteToCache
  const readCachedDocs = vi.fn(() => [] as Array<{ path: string, content: string }>)
  const loadCachedReferences = vi.fn(() => ({ docsToIndex: [] as any[], docSource: 'readme', docsType: 'readme' as const }))
  const detectDocsType = vi.fn(() => ({ docsType: 'readme' as const }))
  const linkAllReferences = vi.fn()
  const ejectReferences = vi.fn()
  const forceClearCache = vi.fn()
  const writeSections = vi.fn()
  const readCachedSection = vi.fn(() => null)
  const isCached = vi.fn(() => false)
  return {
    CACHE_DIR: '/mock-cache',
    REPOS_DIR: '/mock-cache/repos',
    getCacheDir: vi.fn(() => '/mock-cache/references/test-pkg@1.0.0'),
    getPackageDbPath: vi.fn(() => '/mock-cache/references/test-pkg@1.0.0/db'),
    getRepoCacheDir: vi.fn((owner: string, repo: string) => `/mock-cache/repos/${owner}/${repo}`),
    readCachedDocs,
    writeToCache,
    writeToRepoCache: sharedWriteToRepoCache,
    clearCache: sharedClearCache,
    linkCachedDir: vi.fn(),
    linkDiscussions: vi.fn(),
    linkIssues: vi.fn(),
    linkPkg: vi.fn(),
    linkPkgNamed: vi.fn(),
    linkReferences: vi.fn(),
    linkReleases: vi.fn(),
    linkRepoCachedDir: vi.fn(),
    loadCachedReferences,
    detectDocsType,
    linkAllReferences,
    ejectReferences,
    forceClearCache,
    writeSections,
    readCachedSection,
    isCached,
    createReferenceCache: vi.fn((name: string, version: string) => ({
      packageName: name,
      version,
      dir: `/mock-cache/references/${name}@${version}`,
      has: () => isCached(name, version),
      write: (docs: any) => writeToCache(name, version, docs),
      writeSections: (sections: any) => writeSections(name, version, sections),
      readSection: (file: string) => readCachedSection(name, version, file),
      readDocs: () => readCachedDocs(name, version),
      detectDocs: (repoUrl?: string, llmsUrl?: string) => detectDocsType(name, version, repoUrl, llmsUrl),
      load: (opts: any) => loadCachedReferences({ ...opts, packageName: name, version }),
      linkInto: (skillDir: string, cwd: string, docsType: string, o: any) => linkAllReferences(skillDir, name, cwd, version, docsType, o?.extraPackages, o?.features, o?.repoInfo),
      eject: (skillDir: string, cwd: string, docsType: string, o: any) => ejectReferences(skillDir, name, cwd, version, docsType, o?.features, o?.repoInfo),
      clearForce: (repoInfo?: any) => forceClearCache(name, version, repoInfo),
    })),
  }
})

vi.mock('../../src/sources/local-package', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../src/sources/local-package')>()
  return {
    ...actual,
    resolveLocalPackageDocs: vi.fn(),
  }
})

vi.mock('../../src/sources', () => ({
  $fetch: vi.fn(),
  fetchGitHubRaw: vi.fn(),
  downloadLlmsDocs: vi.fn(),
  filterFrameworkDocs: vi.fn((files: string[]) => files),
  fetchBlogReleases: vi.fn(),
  fetchCrawledDocs: vi.fn(() => Promise.resolve([])),
  fetchGitDocs: vi.fn(),
  fetchGitHubDiscussions: vi.fn(),
  fetchGitHubIssues: vi.fn(),
  fetchLlmsTxt: vi.fn(),
  fetchNpmPackage: vi.fn(),
  fetchReadmeContent: vi.fn(),
  fetchReleaseNotes: vi.fn(),
  formatDiscussionAsMarkdown: vi.fn((d: any) => `# ${d.title}`),
  formatIssueAsMarkdown: vi.fn((i: any) => `# ${i.title}`),
  generateDiscussionIndex: vi.fn(() => '# Discussions'),
  generateDocsIndex: vi.fn(() => ''),
  generateIssueIndex: vi.fn(() => '# Issues'),
  generateReleaseIndex: vi.fn(() => '# Releases'),
  getBlogPreset: vi.fn(() => null),
  getPrereleaseChangelogRef: vi.fn(() => undefined),
  isGhAvailable: vi.fn(() => true),
  isPrerelease: vi.fn(() => false),
  isShallowGitDocs: vi.fn(() => false),
  normalizeLlmsLinks: vi.fn((raw: string) => raw),
  parseGitHubUrl: vi.fn((url: string) => {
    const m = url.match(/github\.com\/([^/]+)\/([^/]+)/)
    return m ? { owner: m[1], repo: m[2] } : null
  }),
  resolveEntryFiles: vi.fn(() => []),
  resolveLocalPackageDocs: vi.fn(),
  toCrawlPattern: vi.fn((url: string) => `${url.replace(/\/+$/, '')}/**`),
}))

vi.mock('../../src/core/prepare', () => ({
  getShippedSkills: vi.fn(() => []),
  hasShippedDocs: vi.fn(() => false),
  linkShippedSkill: vi.fn(),
  resolvePkgDir: vi.fn(),
  restorePkgSymlink: vi.fn(),
  getPkgKeyFiles: vi.fn(() => []),
}))

vi.mock('../../src/core/config', () => ({
  readConfig: vi.fn(() => ({ features: { search: true, issues: false, discussions: false, releases: true } })),
  defaultFeatures: { search: true, issues: false, discussions: false, releases: true },
  registerProject: vi.fn(),
}))

vi.mock('../../src/core/sanitize', () => ({
  sanitizeMarkdown: vi.fn((s: string) => s),
}))

vi.mock('../../src/core/lockfile', () => ({
  readLock: vi.fn(() => null),
  writeLock: vi.fn(),
}))

vi.mock('../../src/retriv/index-identity', () => ({
  hasIndexEmbeddingIdentity: vi.fn(() => true),
  resolveEmbeddingIdentity: vi.fn(() => 'v1:model-a@auto'),
}))

vi.mock('../../src/retriv', async (importOriginal) => {
  const orig = await importOriginal<typeof import('../../src/retriv')>()
  return { ...orig, createIndex: vi.fn(), listIndexIds: vi.fn().mockResolvedValue([]) }
})

vi.mock('../../src/agent', () => ({
  agents: {
    'claude-code': { name: 'claude-code', displayName: 'Claude Code', skillsDir: '.claude/skills', globalSkillsDir: '/home/test/.claude/skills' },
  },
}))

const { existsSync, readFileSync, rmSync, mkdirSync, copyFileSync, readdirSync } = await import('node:fs')
const { getCacheDir, getPackageDbPath, readCachedDocs, clearCache } = await import('../../src/cache') as any /* mock surface */
const { fetchReleaseNotes, isGhAvailable, isShallowGitDocs, resolveEntryFiles } = await import('../../src/sources')
const { registerProject } = await import('../../src/core/config')
const { writeLock } = await import('../../src/core/lockfile')
const { createIndex, listIndexIds } = await import('../../src/retriv')
const { hasIndexEmbeddingIdentity, resolveEmbeddingIdentity } = await import('../../src/retriv/index-identity')
const { getShippedSkills, linkShippedSkill, resolvePkgDir } = await import('../../src/core/prepare')

const {
  detectDocsType,
  forceClearCache,
  ejectReferences,
} = await import('../../src/cache/internal/references')
const { classifyCachedDoc } = await import('../../src/cache/internal/classify')
const {
  detectChangelog,
} = await import('../../src/commands/sync/pipeline')
const { resolveLocalDep } = await import('../../src/sources/local-package')
const { indexResources } = await import('../../src/retriv/index-pipeline')
const { handleShippedSkills, resolveBaseDir } = await import('../../src/agent/skill-installer')
const { clearPackageJsonCache } = await import('../../src/core/package-json')

describe('sync-shared', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    clearPackageJsonCache()
    // Restore defaults after reset
    vi.mocked(getCacheDir).mockReturnValue('/mock-cache/references/test-pkg@1.0.0')
    vi.mocked(getPackageDbPath).mockReturnValue('/mock-cache/references/test-pkg@1.0.0/db')
    vi.mocked(readCachedDocs).mockReturnValue([])
    vi.mocked(isGhAvailable).mockReturnValue(true)
    vi.mocked(isShallowGitDocs).mockReturnValue(false)
    vi.mocked(resolveEntryFiles).mockResolvedValue([])
    vi.mocked(fetchReleaseNotes).mockResolvedValue([])
    vi.mocked(hasIndexEmbeddingIdentity).mockReturnValue(true)
    vi.mocked(resolveEmbeddingIdentity).mockReturnValue('v1:model-a@auto')
  })

  // ── 1. classifyCachedDoc ──

  describe('classifyCachedDoc', () => {
    it('classifies issue path', () => {
      expect(classifyCachedDoc('issues/issue-123.md')).toEqual({ type: 'issue', number: 123 })
    })

    it('classifies discussion path', () => {
      expect(classifyCachedDoc('discussions/discussion-42.md')).toEqual({ type: 'discussion', number: 42 })
    })

    it('classifies release path', () => {
      expect(classifyCachedDoc('releases/v3.4.0.md')).toEqual({ type: 'release' })
    })

    it('classifies nested release path', () => {
      expect(classifyCachedDoc('releases/notes/v1.md')).toEqual({ type: 'release' })
    })

    it('classifies doc path', () => {
      expect(classifyCachedDoc('docs/guide/intro.md')).toEqual({ type: 'doc' })
    })

    it('classifies root file as doc', () => {
      expect(classifyCachedDoc('llms.txt')).toEqual({ type: 'doc' })
    })

    it('treats issue without number as doc', () => {
      expect(classifyCachedDoc('issues/issue-.md')).toEqual({ type: 'doc' })
    })

    it('treats issue wrong format as doc', () => {
      expect(classifyCachedDoc('issues/bug-123.md')).toEqual({ type: 'doc' })
    })

    it('treats nested discussion as doc', () => {
      expect(classifyCachedDoc('discussions/old/discussion-1.md')).toEqual({ type: 'doc' })
    })
  })

  // ── 2. detectDocsType ──

  describe('detectDocsType', () => {
    it('returns docs when index.md exists', () => {
      vi.mocked(existsSync).mockImplementation((p: any) =>
        String(p).endsWith('docs/index.md'),
      )
      const result = detectDocsType('pkg', '1.0.0', 'https://github.com/org/repo')
      expect(result.docsType).toBe('docs')
      expect(result.docSource).toContain('tree/v1.0.0/docs')
    })

    it('returns docs when guide dir exists', () => {
      vi.mocked(existsSync).mockImplementation((p: any) =>
        String(p).endsWith('docs/guide'),
      )
      const result = detectDocsType('pkg', '1.0.0')
      expect(result.docsType).toBe('docs')
      expect(result.docSource).toBe('git')
    })

    it('returns llms.txt when llms.txt exists', () => {
      vi.mocked(existsSync).mockImplementation((p: any) =>
        String(p).endsWith('/llms.txt'),
      )
      const result = detectDocsType('pkg', '1.0.0', undefined, 'https://example.com/llms.txt')
      expect(result.docsType).toBe('llms.txt')
      expect(result.docSource).toBe('https://example.com/llms.txt')
    })

    it('returns llms.txt with fallback source', () => {
      vi.mocked(existsSync).mockImplementation((p: any) =>
        String(p).endsWith('/llms.txt'),
      )
      const result = detectDocsType('pkg', '1.0.0')
      expect(result.docsType).toBe('llms.txt')
      expect(result.docSource).toBe('llms.txt')
    })

    it('returns readme when README.md exists', () => {
      vi.mocked(existsSync).mockImplementation((p: any) =>
        String(p).endsWith('docs/README.md'),
      )
      const result = detectDocsType('pkg', '1.0.0')
      expect(result.docsType).toBe('readme')
      expect(result.docSource).toBeUndefined()
    })

    it('returns readme when nothing exists', () => {
      vi.mocked(existsSync).mockReturnValue(false)
      const result = detectDocsType('pkg', '1.0.0')
      expect(result.docsType).toBe('readme')
    })

    it('docs wins over llms.txt when both exist', () => {
      vi.mocked(existsSync).mockImplementation((p: any) => {
        const s = String(p)
        return s.endsWith('docs/index.md') || s.endsWith('/llms.txt')
      })
      const result = detectDocsType('pkg', '1.0.0', 'https://github.com/org/repo', 'https://example.com/llms.txt')
      expect(result.docsType).toBe('docs')
    })
  })

  // ── 3. detectChangelog ──

  describe('detectChangelog', () => {
    it('returns false for null pkgDir', () => {
      expect(detectChangelog(null)).toBe(false)
    })

    it('returns pkg/CHANGELOG.md when it exists', () => {
      vi.mocked(existsSync).mockImplementation((p: any) =>
        String(p).endsWith('CHANGELOG.md'),
      )
      expect(detectChangelog('/pkg')).toBe('pkg/CHANGELOG.md')
    })

    it('returns pkg/changelog.md when only lowercase exists', () => {
      vi.mocked(existsSync).mockImplementation((p: any) =>
        String(p).endsWith('changelog.md') && !String(p).endsWith('CHANGELOG.md'),
      )
      expect(detectChangelog('/pkg')).toBe('pkg/changelog.md')
    })

    it('returns false when neither exists', () => {
      vi.mocked(existsSync).mockReturnValue(false)
      expect(detectChangelog('/pkg')).toBe(false)
    })

    it('detects CHANGELOG.md from cached releases dir', () => {
      vi.mocked(existsSync).mockImplementation((p: any) =>
        String(p).includes('releases/CHANGELOG.md'),
      )
      expect(detectChangelog(null, '/cache')).toBe('releases/CHANGELOG.md')
    })

    it('prefers pkg changelog over cached', () => {
      vi.mocked(existsSync).mockImplementation((p: any) =>
        String(p).endsWith('CHANGELOG.md'),
      )
      expect(detectChangelog('/pkg', '/cache')).toBe('pkg/CHANGELOG.md')
    })
  })

  // ── 4. resolveLocalDep ──

  describe('resolveLocalDep', () => {
    it('returns null when no package.json', async () => {
      vi.mocked(existsSync).mockReturnValue(false)
      expect(await resolveLocalDep('foo', '/cwd')).toBeNull()
    })

    it('returns null for non-link dep', async () => {
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readFileSync).mockReturnValue(JSON.stringify({ dependencies: { foo: '^1.0.0' } }))
      expect(await resolveLocalDep('foo', '/cwd')).toBeNull()
    })

    it('returns null when dep not found', async () => {
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readFileSync).mockReturnValue(JSON.stringify({ dependencies: {} }))
      expect(await resolveLocalDep('foo', '/cwd')).toBeNull()
    })
  })

  // ── 6. indexResources ──

  describe('indexResources', () => {
    const baseOpts = {
      packageName: 'test-pkg',
      version: '1.0.0',
      cwd: '/cwd',
      onProgress: vi.fn(),
    }

    // 6a: db already exists, no changes → reports up to date
    it('reports up to date when db exists and no changes', async () => {
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(listIndexIds).mockResolvedValue(['a.md'])
      vi.mocked(resolvePkgDir).mockReturnValue(null)
      const onProgress = vi.fn()
      await indexResources({ ...baseOpts, docsToIndex: [{ id: 'a.md', content: 'x', metadata: {} }], onProgress })
      expect(createIndex).not.toHaveBeenCalled()
      expect(onProgress).toHaveBeenCalledWith('Search index up to date')
    })

    it('rebuilds every document when embedding settings changed', async () => {
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(hasIndexEmbeddingIdentity).mockReturnValue(false)
      vi.mocked(resolvePkgDir).mockReturnValue(null)
      const docs = [
        { id: 'a.md', content: 'existing', metadata: { type: 'doc' } },
        { id: 'b.md', content: 'existing', metadata: { type: 'doc' } },
      ]

      await indexResources({ ...baseOpts, docsToIndex: docs })

      expect(listIndexIds).not.toHaveBeenCalled()
      expect(createIndex).toHaveBeenCalledWith(docs, expect.objectContaining({ dbPath: expect.any(String) }))
    })

    // 6a2: db exists with new docs → incremental index
    it('incrementally indexes new docs when db exists', async () => {
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(listIndexIds).mockResolvedValue(['a.md'])
      vi.mocked(resolvePkgDir).mockReturnValue(null)
      const docs = [
        { id: 'a.md', content: 'existing', metadata: { type: 'doc' } },
        { id: 'b.md', content: 'new', metadata: { type: 'doc' } },
      ]
      await indexResources({ ...baseOpts, docsToIndex: docs })
      expect(createIndex).toHaveBeenCalled()
      const call = vi.mocked(createIndex).mock.calls[0]
      // Only the new doc should be indexed
      expect(call[0]).toHaveLength(1)
      expect(call[0][0].id).toBe('b.md')
      // No removals
      expect(call[1].removeIds).toEqual([])
    })

    // 6a3: db exists with stale docs → removes them
    it('removes stale docs from existing index', async () => {
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(listIndexIds).mockResolvedValue(['a.md', 'old.md', 'old.md#chunk-0', 'old.md#chunk-1'])
      vi.mocked(resolvePkgDir).mockReturnValue(null)
      const docs = [{ id: 'a.md', content: 'content', metadata: { type: 'doc' } }]
      await indexResources({ ...baseOpts, docsToIndex: docs })
      expect(createIndex).toHaveBeenCalled()
      const call = vi.mocked(createIndex).mock.calls[0]
      // No new docs (a.md already exists)
      expect(call[0]).toHaveLength(0)
      // Stale IDs removed (old.md and its chunks)
      expect(call[1].removeIds).toEqual(['old.md', 'old.md#chunk-0', 'old.md#chunk-1'])
    })

    // 6b: empty docs + no entry files → skips
    it('skips when no docs and no entry files', async () => {
      vi.mocked(existsSync).mockReturnValue(false)
      vi.mocked(resolvePkgDir).mockReturnValue('/pkg')
      await indexResources({ ...baseOpts, docsToIndex: [] })
      expect(createIndex).not.toHaveBeenCalled()
    })

    // 6c: indexes docs
    it('indexes docs', async () => {
      vi.mocked(existsSync).mockReturnValue(false)
      vi.mocked(resolvePkgDir).mockReturnValue(null)
      const docs = [{ id: 'a.md', content: 'content', metadata: { type: 'doc' } }]
      await indexResources({ ...baseOpts, docsToIndex: docs })
      expect(createIndex).toHaveBeenCalledWith(docs, expect.objectContaining({ dbPath: expect.any(String) }))
    })

    // 6d: merges entry files
    it('merges entry files with docs', async () => {
      vi.mocked(existsSync).mockReturnValue(false)
      vi.mocked(resolvePkgDir).mockReturnValue('/pkg')
      vi.mocked(resolveEntryFiles).mockResolvedValue([{ path: 'index.d.ts', content: 'types', type: 'types' }])
      const docs = [{ id: 'a.md', content: 'content', metadata: { type: 'doc' } }]

      await indexResources({ ...baseOpts, docsToIndex: docs })

      expect(createIndex).toHaveBeenCalled()
      const call = vi.mocked(createIndex).mock.calls[0]
      expect(call[0]).toHaveLength(2)
      expect(call[0][1].metadata.source).toBe('pkg/index.d.ts')
    })

    // 6e: features.search=false — skips indexing entirely
    it('skips indexing entirely when search disabled', async () => {
      vi.mocked(existsSync).mockReturnValue(false)
      vi.mocked(resolvePkgDir).mockReturnValue('/pkg')
      const docs = [{ id: 'a.md', content: 'content', metadata: {} }]

      await indexResources({
        ...baseOpts,
        docsToIndex: docs,
        features: { search: false, issues: false, discussions: false, releases: false },
      })

      expect(resolveEntryFiles).not.toHaveBeenCalled()
      expect(createIndex).not.toHaveBeenCalled()
    })

    // 6f: gracefully skips when search deps unavailable
    it('skips indexing when SearchDepsUnavailableError is thrown', async () => {
      const { SearchDepsUnavailableError } = await import('../../src/retriv')
      vi.mocked(existsSync).mockReturnValue(false)
      vi.mocked(resolvePkgDir).mockReturnValue(null)
      vi.mocked(createIndex).mockRejectedValueOnce(new SearchDepsUnavailableError(new Error('mock')))
      const onProgress = vi.fn()
      const docs = [{ id: 'a.md', content: 'content', metadata: { type: 'doc' } }]
      await indexResources({ ...baseOpts, docsToIndex: docs, onProgress })
      expect(onProgress).toHaveBeenCalledWith(expect.stringContaining('skipped'))
    })

    // 6g: re-throws non-SearchDepsUnavailableError errors
    it('re-throws other createIndex errors', async () => {
      vi.mocked(existsSync).mockReturnValue(false)
      vi.mocked(resolvePkgDir).mockReturnValue(null)
      vi.mocked(createIndex).mockRejectedValueOnce(new Error('disk full'))
      const docs = [{ id: 'a.md', content: 'content', metadata: { type: 'doc' } }]
      await expect(indexResources({ ...baseOpts, docsToIndex: docs })).rejects.toThrow('disk full')
    })
  })

  // ── 7. forceClearCache ──

  describe('forceClearCache', () => {
    it('clears cache and removes db when it exists', () => {
      vi.mocked(existsSync).mockReturnValue(true)
      forceClearCache('pkg', '1.0.0')
      expect(clearCache).toHaveBeenCalledWith('pkg', '1.0.0')
      expect(rmSync).toHaveBeenCalled()
    })

    it('clears cache but skips rmSync when db missing', () => {
      vi.mocked(existsSync).mockReturnValue(false)
      forceClearCache('pkg', '1.0.0')
      expect(clearCache).toHaveBeenCalledWith('pkg', '1.0.0')
      expect(rmSync).not.toHaveBeenCalled()
    })
  })

  // ── 8. handleShippedSkills ──

  describe('handleShippedSkills', () => {
    it('returns null when no shipped skills', () => {
      vi.mocked(getShippedSkills).mockReturnValue([])
      expect(handleShippedSkills('pkg', '1.0.0', '/cwd', 'claude-code', false)).toBeNull()
    })

    it('links shipped skills and registers project', () => {
      vi.mocked(getShippedSkills).mockReturnValue([{ skillName: 'my-skill', skillDir: '/shipped' }])
      const result = handleShippedSkills('pkg', '1.0.0', '/cwd', 'claude-code', false)
      expect(result).not.toBeNull()
      expect(linkShippedSkill).toHaveBeenCalled()
      expect(writeLock).toHaveBeenCalled()
      expect(registerProject).toHaveBeenCalledWith('/cwd')
    })

    it('does not register project when global', () => {
      vi.mocked(getShippedSkills).mockReturnValue([{ skillName: 'my-skill', skillDir: '/shipped' }])
      handleShippedSkills('pkg', '1.0.0', '/cwd', 'claude-code', true)
      expect(registerProject).not.toHaveBeenCalled()
    })
  })

  // ── 9. resolveBaseDir ──

  describe('resolveBaseDir', () => {
    it('returns project skills dir when not global', () => {
      expect(resolveBaseDir('/cwd', 'claude-code', false)).toContain('.claude/skills')
    })

    it('returns cache skills dir when global', () => {
      expect(resolveBaseDir('/cwd', 'claude-code', true)).toContain('skills')
    })
  })

  // ── 10. ejectReferences ──

  describe('ejectReferences', () => {
    beforeEach(() => {
      vi.resetAllMocks()
      clearPackageJsonCache()
      vi.mocked(getCacheDir).mockReturnValue('/mock-cache/references/vue@3.4.0')
    })

    it('copies cached docs to references/docs/', () => {
      vi.mocked(existsSync).mockImplementation((p: any) => {
        const s = String(p)
        return s.includes('/docs') && !s.includes('shipped')
      })
      vi.mocked(readdirSync).mockReturnValue([
        { name: 'guide.md', isDirectory: () => false, isFile: () => true, isBlockDevice: () => false, isCharacterDevice: () => false, isFIFO: () => false, isSocket: () => false, isSymbolicLink: () => false, parentPath: '', path: '' },
      ] as any)
      vi.mocked(resolvePkgDir).mockReturnValue(null)

      ejectReferences('/skill', 'vue', '/cwd', '3.4.0', 'docs')

      expect(mkdirSync).toHaveBeenCalledWith(expect.stringContaining('references/docs'), expect.anything())
      expect(copyFileSync).toHaveBeenCalled()
    })

    it('copies issues when feature enabled', () => {
      vi.mocked(existsSync).mockImplementation((p: any) => {
        const s = String(p)
        return s.includes('/issues')
      })
      vi.mocked(readdirSync).mockReturnValue([
        { name: '_INDEX.md', isDirectory: () => false, isFile: () => true, isBlockDevice: () => false, isCharacterDevice: () => false, isFIFO: () => false, isSocket: () => false, isSymbolicLink: () => false, parentPath: '', path: '' },
      ] as any)
      vi.mocked(resolvePkgDir).mockReturnValue(null)

      ejectReferences('/skill', 'vue', '/cwd', '3.4.0', 'docs', { search: false, issues: true, discussions: false, releases: false })

      expect(mkdirSync).toHaveBeenCalledWith(expect.stringContaining('references/issues'), expect.anything())
      expect(copyFileSync).toHaveBeenCalled()
    })

    it('skips docs when docsType is readme', () => {
      vi.mocked(existsSync).mockReturnValue(false)
      vi.mocked(resolvePkgDir).mockReturnValue(null)

      ejectReferences('/skill', 'vue', '/cwd', '3.4.0', 'readme')

      expect(copyFileSync).not.toHaveBeenCalled()
    })

    it('does not copy pkg files', () => {
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readdirSync).mockReturnValue([])
      vi.mocked(resolvePkgDir).mockReturnValue('/node_modules/vue')

      ejectReferences('/skill', 'vue', '/cwd', '3.4.0', 'docs')

      // Should not create a pkg directory or copy package.json
      const mkdirCalls = vi.mocked(mkdirSync).mock.calls.map(c => String(c[0]))
      expect(mkdirCalls.some(c => c.includes('references/pkg'))).toBe(false)
    })
  })
})
