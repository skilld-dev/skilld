import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getCacheKey, getVersionKey } from '../../src/cache'

// Mock fs for file operation tests
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
  }
})

describe('cache', () => {
  describe('getVersionKey', () => {
    it('returns exact version', () => {
      expect(getVersionKey('1.2.3')).toBe('1.2.3')
      expect(getVersionKey('10.20.30')).toBe('10.20.30')
      expect(getVersionKey('0.0.1')).toBe('0.0.1')
    })

    it('returns prerelease versions as-is', () => {
      expect(getVersionKey('1.2.3-beta.1')).toBe('1.2.3-beta.1')
      expect(getVersionKey('2.0.0-rc.1')).toBe('2.0.0-rc.1')
    })

    it('returns original if no match', () => {
      expect(getVersionKey('latest')).toBe('latest')
      expect(getVersionKey('next')).toBe('next')
    })
  })

  describe('getCacheKey', () => {
    it('combines name and version key', () => {
      expect(getCacheKey('vue', '3.4.21')).toBe('vue@3.4.21')
      expect(getCacheKey('@nuxt/kit', '3.10.0')).toBe('@nuxt/kit@3.10.0')
    })
  })

  describe('createReferenceCache().has()', () => {
    beforeEach(() => {
      vi.resetAllMocks()
    })

    afterEach(() => {
      vi.restoreAllMocks()
    })

    it('returns true when cache dir exists', async () => {
      const { existsSync } = await import('node:fs')
      const { createReferenceCache } = await import('../../src/cache')
      vi.mocked(existsSync).mockReturnValue(true)

      expect(createReferenceCache('vue', '3.4.0').has()).toBe(true)
    })

    it('returns false when cache dir missing', async () => {
      const { existsSync } = await import('node:fs')
      const { createReferenceCache } = await import('../../src/cache')
      vi.mocked(existsSync).mockReturnValue(false)

      expect(createReferenceCache('vue', '3.4.0').has()).toBe(false)
    })
  })

  describe('listCachedPackages', () => {
    beforeEach(() => {
      vi.resetAllMocks()
    })

    it('returns empty array when references dir missing', async () => {
      const { existsSync } = await import('node:fs')
      const { listCachedPackages } = await import('../../src/cache')
      vi.mocked(existsSync).mockReturnValue(false)

      expect(listCachedPackages()).toEqual([])
    })

    it('parses cached package entries', async () => {
      const { existsSync, readdirSync } = await import('node:fs')
      const { listCachedPackages } = await import('../../src/cache')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readdirSync).mockReturnValue(['vue@3.4', 'nuxt@3.10'] as any)

      const result = listCachedPackages()

      expect(result).toHaveLength(2)
      expect(result[0]).toMatchObject({ name: 'vue', version: '3.4' })
      expect(result[1]).toMatchObject({ name: 'nuxt', version: '3.10' })
    })

    it('filters entries without @', async () => {
      const { existsSync, readdirSync } = await import('node:fs')
      const { listCachedPackages } = await import('../../src/cache')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readdirSync).mockReturnValue(['vue@3.4', '.DS_Store', 'random'] as any)

      const result = listCachedPackages()
      expect(result).toHaveLength(1)
    })
  })

  describe('createReferenceCache().write()', () => {
    beforeEach(() => {
      vi.resetAllMocks()
    })

    it('creates cache dir and writes docs', async () => {
      const { mkdirSync, writeFileSync } = await import('node:fs')
      const { createReferenceCache } = await import('../../src/cache')

      createReferenceCache('vue', '3.4.0').write([
        { path: 'README.md', content: '# Vue' },
        { path: 'api/core.md', content: '# Core API' },
      ])

      expect(mkdirSync).toHaveBeenCalled()
      expect(writeFileSync).toHaveBeenCalledTimes(2)
    })
  })
})
