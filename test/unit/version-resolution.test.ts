import type { SkillEntry } from '../../src/core/skills.ts'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { isOutdated } from '../../src/core/skills.ts'

// Mock ofetch
const mockFetch = vi.fn()

function createMockFetch() {
  async function $fetch(url: string, opts?: any): Promise<any> {
    const mockRes = await mockFetch(url, opts)
    if (!mockRes?.ok)
      throw new Error('fetch failed')
    if (opts?.responseType === 'text')
      return mockRes.text()
    return mockRes.json()
  }
  $fetch.raw = async (url: string, opts?: any) => mockFetch(url, opts)
  return $fetch
}

vi.mock('ofetch', () => ({
  ofetch: { create: () => createMockFetch() },
}))

vi.mock('mlly', () => ({
  resolvePathSync: vi.fn(),
}))

vi.mock('node:fs', async () => {
  const actual = await vi.importActual<typeof import('node:fs')>('node:fs')
  return {
    ...actual,
    existsSync: vi.fn(),
    readFileSync: vi.fn(),
  }
})

const { fetchLatestVersion, fetchNpmRegistryMeta } = await import('../../src/sources/npm-registry')
const { parseVersionSpecifier, resolveInstalledVersion } = await import('../../src/sources/local-package')
const { clearPackageJsonCache } = await import('../../src/core/package-json')

function makeSkill(version: string | undefined): SkillEntry {
  return {
    name: 'test-pkg',
    dir: '/test',
    agent: 'claude-code',
    info: version ? { version, generator: 'skilld', packageName: 'test-pkg' } : null,
    scope: 'local',
  }
}

describe('version resolution stability gaps', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    clearPackageJsonCache()
  })

  describe('fetchLatestVersion - no fallback when unpkg fails', () => {
    it('returns null only when both unpkg and registry fail', async () => {
      mockFetch.mockRejectedValueOnce(new Error('unpkg is down'))
      mockFetch.mockRejectedValueOnce(new Error('registry is down'))

      const result = await fetchLatestVersion('vue')

      expect(result).toBeNull()
      // Both sources attempted
      expect(mockFetch).toHaveBeenCalledTimes(2)
      expect(mockFetch).toHaveBeenNthCalledWith(1, 'https://unpkg.com/vue/package.json', undefined)
      expect(mockFetch).toHaveBeenNthCalledWith(2, 'https://registry.npmjs.org/vue', { headers: { Accept: 'application/vnd.npm.install-v1+json' } })
    })

    it('falls back to npm registry when unpkg fails', async () => {
      // unpkg fails
      mockFetch.mockResolvedValueOnce({ ok: false })
      // registry succeeds
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ 'dist-tags': { latest: '3.5.0' } }),
      })

      const result = await fetchLatestVersion('vue')

      expect(result).toBe('3.5.0')
      expect(mockFetch).toHaveBeenCalledTimes(2)
    })
  })

  describe('fetchNpmRegistryMeta - fetches full document for just dist-tags', () => {
    it('uses abbreviated metadata header', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          'dist-tags': { latest: '3.5.0', next: '4.0.0-alpha.1' },
          'time': { '3.5.0': '2024-06-01T00:00:00Z' },
        }),
      })

      await fetchNpmRegistryMeta('vue', '3.5.0')

      // Should use Accept header for abbreviated metadata
      const [url, opts] = mockFetch.mock.calls[0]
      expect(url).toBe('https://registry.npmjs.org/vue')
      // Check that abbreviated metadata header is sent
      expect(opts?.headers?.Accept || opts?.headers?.accept).toBe(
        'application/vnd.npm.install-v1+json',
      )
    })
  })

  describe('isOutdated - prefix stripping inconsistencies', () => {
    it('handles >= prefix (currently only strips ^ and ~)', () => {
      // If dep version is ">=2.0.0", isOutdated should still compare correctly
      expect(isOutdated(makeSkill('1.0.0'), '>=2.0.0')).toBe(true)
    })

    it('handles complex range like >=1.0.0 <2.0.0', () => {
      // Complex ranges should not crash
      expect(isOutdated(makeSkill('1.0.0'), '>=1.0.0 <2.0.0')).toBe(false)
    })
  })

  describe('parseVersionSpecifier - catalog/workspace fallback', () => {
    it('catalog: without node_modules falls back to wildcard', async () => {
      const { resolvePathSync } = await import('mlly')
      vi.mocked(resolvePathSync).mockImplementation(() => {
        throw new Error('not found')
      })

      const result = parseVersionSpecifier('some-pkg', 'catalog:deps', '/test')

      // Currently falls to * — ideally would resolve from lockfile
      expect(result).toEqual({ name: 'some-pkg', version: '*' })
    })

    it('workspace: without node_modules falls back to wildcard', async () => {
      const { resolvePathSync } = await import('mlly')
      vi.mocked(resolvePathSync).mockImplementation(() => {
        throw new Error('not found')
      })

      const result = parseVersionSpecifier('some-pkg', 'workspace:*', '/test')

      expect(result).toEqual({ name: 'some-pkg', version: '*' })
    })

    it('catalog: resolves from node_modules when available', async () => {
      const { resolvePathSync } = await import('mlly')
      const { existsSync, readFileSync } = await import('node:fs')
      vi.mocked(resolvePathSync).mockReturnValue('/test/node_modules/some-pkg/package.json')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readFileSync).mockReturnValue(JSON.stringify({ version: '2.1.0' }))

      const result = parseVersionSpecifier('some-pkg', 'catalog:deps', '/test')

      expect(result).toEqual({ name: 'some-pkg', version: '2.1.0' })
    })
  })

  describe('resolveInstalledVersion - edge cases', () => {
    it('handles scoped packages correctly', async () => {
      const { resolvePathSync } = await import('mlly')
      const { existsSync, readFileSync } = await import('node:fs')
      vi.mocked(resolvePathSync).mockReturnValue('/test/node_modules/@vue/compiler-core/package.json')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readFileSync).mockReturnValue(JSON.stringify({ version: '3.4.0' }))

      const result = resolveInstalledVersion('@vue/compiler-core', '/test')

      expect(result).toBe('3.4.0')
    })

    it('handles package.json with no version field', async () => {
      const { resolvePathSync } = await import('mlly')
      const { readFileSync } = await import('node:fs')
      vi.mocked(resolvePathSync).mockReturnValue('/test/node_modules/pkg/package.json')
      vi.mocked(readFileSync).mockReturnValue(JSON.stringify({ name: 'pkg' }))

      const result = resolveInstalledVersion('pkg', '/test')

      expect(result).toBeNull()
    })
  })
})
