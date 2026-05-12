import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// Mock fs module
vi.mock('node:fs', async () => {
  const actual = await vi.importActual<typeof import('node:fs')>('node:fs')
  return {
    ...actual,
    existsSync: vi.fn(),
    lstatSync: vi.fn(),
    mkdirSync: vi.fn(),
    writeFileSync: vi.fn(),
    readFileSync: vi.fn(),
    readdirSync: vi.fn(),
    unlinkSync: vi.fn(),
    symlinkSync: vi.fn(),
    rmSync: vi.fn(),
  }
})

describe('cache/storage', () => {
  beforeEach(() => {
    vi.resetAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('isCached', () => {
    it('returns true when cache dir exists', async () => {
      const { existsSync } = await import('node:fs')
      const { isCached } = await import('../../src/cache/internal/storage')
      vi.mocked(existsSync).mockReturnValue(true)

      expect(isCached('vue', '3.4.0')).toBe(true)
    })

    it('returns false when cache dir missing', async () => {
      const { existsSync } = await import('node:fs')
      const { isCached } = await import('../../src/cache/internal/storage')
      vi.mocked(existsSync).mockReturnValue(false)

      expect(isCached('vue', '3.4.0')).toBe(false)
    })
  })

  describe('inferDocsTypeFromCache', () => {
    it('detects llms.txt from source', async () => {
      const { inferDocsTypeFromCache } = await import('../../src/cache/internal/storage')

      expect(inferDocsTypeFromCache('/cache/vue', 'llms.txt')).toBe('llms.txt')
    })

    it('detects cached llms.txt docs', async () => {
      const { existsSync } = await import('node:fs')
      const { inferDocsTypeFromCache } = await import('../../src/cache/internal/storage')
      vi.mocked(existsSync).mockImplementation(path => String(path).endsWith('docs/llms.txt'))

      expect(inferDocsTypeFromCache('/cache/vue')).toBe('llms.txt')
    })

    it('detects README-only cache', async () => {
      const { existsSync, readdirSync } = await import('node:fs')
      const { inferDocsTypeFromCache } = await import('../../src/cache/internal/storage')
      vi.mocked(existsSync).mockImplementation(path => String(path).endsWith('docs'))
      vi.mocked(readdirSync).mockReturnValue(['README.md'] as any)

      expect(inferDocsTypeFromCache('/cache/vue')).toBe('readme')
    })

    it('defaults to docs', async () => {
      const { existsSync, readdirSync } = await import('node:fs')
      const { inferDocsTypeFromCache } = await import('../../src/cache/internal/storage')
      vi.mocked(existsSync).mockImplementation(path => String(path).endsWith('docs'))
      vi.mocked(readdirSync).mockReturnValue(['README.md', 'guide.md'] as any)

      expect(inferDocsTypeFromCache('/cache/vue')).toBe('docs')
    })
  })

  describe('ensureCacheDir', () => {
    it('creates references directory', async () => {
      const { mkdirSync } = await import('node:fs')
      const { ensureCacheDir } = await import('../../src/cache/internal/storage')

      ensureCacheDir()

      expect(mkdirSync).toHaveBeenCalledWith(
        expect.stringContaining('references'),
        { recursive: true, mode: 0o700 },
      )
    })
  })

  describe('writeToCache', () => {
    it('creates cache dir and writes docs', async () => {
      const { mkdirSync, writeFileSync } = await import('node:fs')
      const { writeToCache } = await import('../../src/cache/internal/storage')

      const result = writeToCache('vue', '3.4.0', [
        { path: 'README.md', content: '# Vue' },
        { path: 'api/core.md', content: '# Core API' },
      ])

      expect(mkdirSync).toHaveBeenCalled()
      expect(writeFileSync).toHaveBeenCalledTimes(2)
      expect(result).toContain('vue@3.4')
    })
  })

  describe('listCached', () => {
    it('returns empty array when references dir missing', async () => {
      const { existsSync } = await import('node:fs')
      const { listCached } = await import('../../src/cache/internal/storage')
      vi.mocked(existsSync).mockReturnValue(false)

      expect(listCached()).toEqual([])
    })

    it('parses cached package entries', async () => {
      const { existsSync, readdirSync } = await import('node:fs')
      const { listCached } = await import('../../src/cache/internal/storage')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readdirSync).mockReturnValue(['vue@3.4', 'nuxt@3.10'] as any)

      const result = listCached()

      expect(result).toHaveLength(2)
      expect(result[0]).toMatchObject({ name: 'vue', version: '3.4' })
      expect(result[1]).toMatchObject({ name: 'nuxt', version: '3.10' })
    })

    it('parses scoped package entries', async () => {
      const { existsSync, readdirSync } = await import('node:fs')
      const { listCached } = await import('../../src/cache/internal/storage')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readdirSync).mockReturnValue(['@vue/reactivity@3.5.0', '@nuxtjs/tailwindcss@6.12.0'] as any)

      const result = listCached()

      expect(result).toHaveLength(2)
      expect(result[0]).toMatchObject({ name: '@vue/reactivity', version: '3.5.0' })
      expect(result[1]).toMatchObject({ name: '@nuxtjs/tailwindcss', version: '6.12.0' })
    })

    it('filters entries without @', async () => {
      const { existsSync, readdirSync } = await import('node:fs')
      const { listCached } = await import('../../src/cache/internal/storage')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readdirSync).mockReturnValue(['vue@3.4', '.DS_Store', 'random'] as any)

      const result = listCached()
      expect(result).toHaveLength(1)
    })
  })

  describe('readCachedDocs', () => {
    it('returns empty array when cache dir missing', async () => {
      const { existsSync } = await import('node:fs')
      const { readCachedDocs } = await import('../../src/cache/internal/storage')
      vi.mocked(existsSync).mockReturnValue(false)

      expect(readCachedDocs('vue', '3.4.0')).toEqual([])
    })

    it('walks directory and reads .md files', async () => {
      const { existsSync, readdirSync, readFileSync } = await import('node:fs')
      const { readCachedDocs } = await import('../../src/cache/internal/storage')

      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readdirSync).mockReturnValue([
        { name: 'README.md', isDirectory: () => false },
        { name: 'index.js', isDirectory: () => false },
      ] as any)
      vi.mocked(readFileSync).mockReturnValue('# Content')

      const docs = readCachedDocs('vue', '3.4.0')

      expect(docs).toHaveLength(1)
      expect(docs[0].path).toBe('README.md')
      expect(docs[0].content).toBe('# Content')
    })

    it('recursively walks subdirectories', async () => {
      const { existsSync, readdirSync, readFileSync } = await import('node:fs')
      const { readCachedDocs } = await import('../../src/cache/internal/storage')

      vi.mocked(existsSync).mockReturnValue(true)
      let callCount = 0
      vi.mocked(readdirSync).mockImplementation(() => {
        callCount++
        if (callCount === 1) {
          return [
            { name: 'api', isDirectory: () => true },
            { name: 'README.md', isDirectory: () => false },
          ] as any
        }
        return [
          { name: 'core.md', isDirectory: () => false },
        ] as any
      })
      vi.mocked(readFileSync).mockReturnValue('content')

      const docs = readCachedDocs('vue', '3.4.0')

      expect(docs).toHaveLength(2)
      expect(docs.map(d => d.path)).toContain('README.md')
      expect(docs.map(d => d.path)).toContain('api/core.md')
    })
  })

  describe('clearCache', () => {
    it('returns false when cache does not exist', async () => {
      const { existsSync } = await import('node:fs')
      const { clearCache } = await import('../../src/cache/internal/storage')
      vi.mocked(existsSync).mockReturnValue(false)

      expect(clearCache('vue', '3.4.0')).toBe(false)
    })

    // Note: clearCache with existing cache uses require('node:fs').rmSync
    // which is difficult to mock - integration test would be better
  })

  describe('clearAllCachedPackages', () => {
    it('returns 0 when no packages cached', async () => {
      const { existsSync } = await import('node:fs')
      const { clearAllCachedPackages } = await import('../../src/cache/registry')
      vi.mocked(existsSync).mockReturnValue(false)

      expect(clearAllCachedPackages()).toBe(0)
    })
  })
})
