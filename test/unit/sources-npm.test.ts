import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// Mock ofetch — simulates ofetch behavior using mockFetch
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
  $fetch.raw = async (url: string, opts?: any) => {
    return mockFetch(url, opts)
  }
  return $fetch
}

vi.mock('ofetch', () => ({
  ofetch: { create: () => createMockFetch() },
}))

// Mock the github and llms modules
vi.mock('../../src/sources/github', () => ({
  fetchGitHubRepoMeta: vi.fn(),
  fetchReadme: vi.fn(),
  fetchGitDocs: vi.fn(),
  searchGitHubRepo: vi.fn(),
}))

vi.mock('../../src/sources/llms', () => ({
  fetchLlmsUrl: vi.fn(),
}))

// Mock mlly
vi.mock('mlly', () => ({
  resolvePathSync: vi.fn(),
}))

// Mock fs module
vi.mock('node:fs', async () => {
  const actual = await vi.importActual<typeof import('node:fs')>('node:fs')
  return {
    ...actual,
    existsSync: vi.fn(),
    readFileSync: vi.fn(),
    createWriteStream: vi.fn(),
    mkdirSync: vi.fn(),
    rmSync: vi.fn(),
  }
})

// Mock child_process for spawnSync
vi.mock('node:child_process', async () => {
  const actual = await vi.importActual<typeof import('node:child_process')>('node:child_process')
  return {
    ...actual,
    spawnSync: vi.fn(),
  }
})

// Must import after vi.mock
const { fetchNpmPackage, fetchPkgDist } = await import('../../src/sources/npm-registry')
const { getInstalledSkillVersion, readLocalDependencies, resolveInstalledVersion } = await import('../../src/sources/local-package')
const { resolvePackageDocs } = await import('../../src/sources/resolver')
const { clearPackageJsonCache } = await import('../../src/core/package-json')

describe('sources/npm', () => {
  beforeEach(() => {
    clearPackageJsonCache()
  })

  describe('readLocalDependencies', () => {
    beforeEach(() => {
      vi.resetAllMocks()
    })

    afterEach(() => {
      vi.restoreAllMocks()
    })

    it('resolves actual installed versions via mlly', async () => {
      const { existsSync, readFileSync } = await import('node:fs')
      const { resolvePathSync } = await import('mlly')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readFileSync).mockImplementation((p: any) => {
        if (String(p).endsWith('package.json') && String(p).includes('node_modules'))
          return JSON.stringify({ version: '3.4.21' })
        return JSON.stringify({
          dependencies: { vue: '^3.4.0', pinia: '~2.1.0' },
          devDependencies: { vitest: '^1.0.0' },
        })
      })
      vi.mocked(resolvePathSync).mockImplementation((id: string) => `/test/node_modules/${id}`)

      const deps = await readLocalDependencies('/test')

      expect(deps).toContainEqual({ name: 'vue', version: '3.4.21' })
      expect(deps).toContainEqual({ name: 'pinia', version: '3.4.21' })
    })

    it('falls back to stripping semver prefix when module not installed', async () => {
      const { existsSync, readFileSync } = await import('node:fs')
      const { resolvePathSync } = await import('mlly')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readFileSync).mockReturnValue(JSON.stringify({
        dependencies: {
          'pkg-caret': '^1.0.0',
          'pkg-tilde': '~2.0.0',
          'pkg-exact': '4.0.0',
          'pkg-gte': '>=1.5.0',
          'pkg-lte': '<=3.0.0',
        },
      }))
      vi.mocked(resolvePathSync).mockImplementation(() => {
        throw new Error('not found')
      })

      const deps = await readLocalDependencies('/test')

      expect(deps).toContainEqual({ name: 'pkg-caret', version: '1.0.0' })
      expect(deps).toContainEqual({ name: 'pkg-tilde', version: '2.0.0' })
      expect(deps).toContainEqual({ name: 'pkg-exact', version: '4.0.0' })
      expect(deps).toContainEqual({ name: 'pkg-gte', version: '1.5.0' })
      expect(deps).toContainEqual({ name: 'pkg-lte', version: '3.0.0' })
    })

    it('throws if package.json not found', async () => {
      const { existsSync } = await import('node:fs')
      vi.mocked(existsSync).mockReturnValue(false)

      await expect(readLocalDependencies('/test'))
        .rejects
        .toThrow('No package.json found')
    })

    it('resolves catalog: and workspace: via installed version', async () => {
      const { existsSync, readFileSync } = await import('node:fs')
      const { resolvePathSync } = await import('mlly')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readFileSync).mockImplementation((p: any) => {
        const s = String(p)
        if (s.includes('node_modules/@clack/prompts'))
          return JSON.stringify({ version: '1.0.0' })
        if (s.includes('node_modules/citty'))
          return JSON.stringify({ version: '0.2.1' })
        if (s.includes('node_modules/bumpp'))
          return JSON.stringify({ version: '10.4.1' })
        return JSON.stringify({
          dependencies: {
            '@clack/prompts': 'catalog:deps',
            'citty': 'catalog:deps',
            'bumpp': 'workspace:*',
          },
        })
      })
      vi.mocked(resolvePathSync).mockImplementation((id: string) => `/test/node_modules/${id}`)

      const deps = await readLocalDependencies('/test')

      expect(deps).toContainEqual({ name: '@clack/prompts', version: '1.0.0' })
      expect(deps).toContainEqual({ name: 'citty', version: '0.2.1' })
      expect(deps).toContainEqual({ name: 'bumpp', version: '10.4.1' })
    })

    it('resolves catalog: specifiers with wildcard when node_modules lookup fails', async () => {
      const { existsSync, readFileSync } = await import('node:fs')
      const { resolvePathSync } = await import('mlly')
      vi.mocked(existsSync).mockImplementation((p) => {
        // package.json exists, but node_modules/<pkg>/package.json does not
        return String(p) === '/test/package.json'
      })
      vi.mocked(readFileSync).mockReturnValue(JSON.stringify({
        dependencies: { 'some-pkg': 'catalog:deps' },
      }))
      vi.mocked(resolvePathSync).mockImplementation(() => {
        throw new Error('not found')
      })

      const deps = await readLocalDependencies('/test')

      expect(deps).toHaveLength(1)
      expect(deps[0]).toEqual({ name: 'some-pkg', version: '*' })
    })

    it('returns null for unresolvable non-standard specifiers', async () => {
      const { existsSync, readFileSync } = await import('node:fs')
      const { resolvePathSync } = await import('mlly')
      vi.mocked(existsSync).mockImplementation((p) => {
        return String(p) === '/test/package.json'
      })
      vi.mocked(readFileSync).mockReturnValue(JSON.stringify({
        dependencies: { 'some-pkg': 'custom:something' },
      }))
      vi.mocked(resolvePathSync).mockImplementation(() => {
        throw new Error('not found')
      })

      const deps = await readLocalDependencies('/test')

      expect(deps).toHaveLength(0)
    })
  })

  describe('resolveInstalledVersion', () => {
    beforeEach(() => {
      vi.resetAllMocks()
    })

    afterEach(() => {
      vi.restoreAllMocks()
    })

    it('resolves version from installed package.json', async () => {
      const { existsSync, readFileSync } = await import('node:fs')
      const { resolvePathSync } = await import('mlly')
      vi.mocked(resolvePathSync).mockReturnValue('/project/node_modules/vue/package.json')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readFileSync).mockReturnValue(JSON.stringify({ version: '3.4.21' }))

      expect(resolveInstalledVersion('vue', '/project')).toBe('3.4.21')
      expect(vi.mocked(resolvePathSync)).toHaveBeenCalledWith('vue/package.json', { url: '/project' })
    })

    it('returns null when package not installed', async () => {
      const { resolvePathSync } = await import('mlly')
      vi.mocked(resolvePathSync).mockImplementation(() => {
        throw new Error('not found')
      })

      expect(resolveInstalledVersion('nonexistent', '/project')).toBeNull()
    })

    it('terminates at filesystem root instead of looping forever', async () => {
      const { resolvePathSync } = await import('mlly')
      const { existsSync } = await import('node:fs')
      // First call (package.json path) throws, triggering fallback walk-up
      vi.mocked(resolvePathSync)
        .mockImplementationOnce(() => { throw new Error('no exports') })
        .mockReturnValueOnce('/some/deep/path/entry.js')
      // No package.json exists anywhere in the tree
      vi.mocked(existsSync).mockReturnValue(false)

      expect(resolveInstalledVersion('pkg', '/some/deep/path')).toBeNull()
    })
  })

  describe('fetchNpmPackage', () => {
    beforeEach(() => {
      vi.resetAllMocks()
    })

    it('fetches package info from npm registry', async () => {
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          name: 'vue',
          version: '3.4.21',
          description: 'Progressive framework',
        }),
      })

      const result = await fetchNpmPackage('vue')

      expect(result).toEqual({
        name: 'vue',
        version: '3.4.21',
        description: 'Progressive framework',
      })
      expect(mockFetch).toHaveBeenCalledWith(
        'https://unpkg.com/vue/package.json',
        undefined,
      )
    })

    it('returns null on fetch error', async () => {
      mockFetch.mockRejectedValueOnce(new Error('Network error'))
      mockFetch.mockRejectedValueOnce(new Error('Network error'))

      const result = await fetchNpmPackage('nonexistent')

      expect(result).toBeNull()
    })

    it('returns null on non-ok response', async () => {
      mockFetch.mockResolvedValueOnce({ ok: false })
      mockFetch.mockResolvedValueOnce({ ok: false })

      const result = await fetchNpmPackage('nonexistent')

      expect(result).toBeNull()
    })
  })

  describe('getInstalledSkillVersion', () => {
    beforeEach(() => {
      vi.resetAllMocks()
    })

    it('returns null when SKILL.md does not exist', async () => {
      const { existsSync } = await import('node:fs')
      vi.mocked(existsSync).mockReturnValue(false)

      const result = await getInstalledSkillVersion('/skills/vue')

      expect(result).toBeNull()
    })

    it('extracts version from SKILL.md frontmatter', async () => {
      const { existsSync, readFileSync } = await import('node:fs')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readFileSync).mockReturnValue(`---
name: vue
version: "3.4.21"
description: Vue skill
---

# Vue`)

      const result = await getInstalledSkillVersion('/skills/vue')

      expect(result).toBe('3.4.21')
    })

    it('handles version without quotes', async () => {
      const { existsSync, readFileSync } = await import('node:fs')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readFileSync).mockReturnValue(`---
name: vue
version: 3.4.21
---`)

      const result = await getInstalledSkillVersion('/skills/vue')

      expect(result).toBe('3.4.21')
    })

    it('returns null when no version field', async () => {
      const { existsSync, readFileSync } = await import('node:fs')
      vi.mocked(existsSync).mockReturnValue(true)
      vi.mocked(readFileSync).mockReturnValue(`---
name: vue
description: Vue skill
---`)

      const result = await getInstalledSkillVersion('/skills/vue')

      expect(result).toBeNull()
    })
  })

  describe('resolvePackageDocs', () => {
    beforeEach(() => {
      vi.resetAllMocks()
    })

    it('returns null when package not found', async () => {
      mockFetch.mockRejectedValueOnce(new Error('Not found'))
      mockFetch.mockRejectedValueOnce(new Error('Not found'))

      const result = await resolvePackageDocs('nonexistent')

      expect(result).toBeNull()
    })

    it('extracts basic info from npm package', async () => {
      const { fetchGitHubRepoMeta, fetchReadme } = await import('../../src/sources/github')
      const { fetchLlmsUrl } = await import('../../src/sources/llms')

      // First fetch: package info
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          name: 'vue',
          version: '3.4.21',
          description: 'Progressive framework',
          homepage: 'https://vuejs.org',
          repository: { url: 'git+https://github.com/vuejs/core.git' },
        }),
      })
      // Second fetch: package time info for release date
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ time: { '3.4.21': '2024-02-01T12:00:00Z' } }),
      })

      vi.mocked(fetchLlmsUrl).mockResolvedValue('https://vuejs.org/llms.txt')
      vi.mocked(fetchGitHubRepoMeta).mockResolvedValue(null)
      vi.mocked(fetchReadme).mockResolvedValue(null)

      const result = await resolvePackageDocs('vue')

      expect(result).toMatchObject({
        name: 'vue',
        version: '3.4.21',
        releasedAt: '2024-02-01T12:00:00Z',
        description: 'Progressive framework',
        docsUrl: 'https://vuejs.org',
        repoUrl: 'https://github.com/vuejs/core',
        llmsUrl: 'https://vuejs.org/llms.txt',
      })
    })

    it('skips homepage if its a social media URL', async () => {
      const { fetchGitHubRepoMeta, fetchReadme } = await import('../../src/sources/github')
      const { fetchLlmsUrl } = await import('../../src/sources/llms')

      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          name: 'autoprefixer',
          version: '10.4.0',
          homepage: 'https://twitter.com/autoprefixer',
          repository: { url: 'https://github.com/postcss/autoprefixer' },
        }),
      })
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ time: { '10.4.0': '2024-01-01T00:00:00Z' } }),
      })

      vi.mocked(fetchGitHubRepoMeta).mockResolvedValue({ homepage: 'https://autoprefixer.github.io' })
      vi.mocked(fetchReadme).mockResolvedValue('ungh://postcss/autoprefixer')
      vi.mocked(fetchLlmsUrl).mockResolvedValue(null)

      const result = await resolvePackageDocs('autoprefixer')

      // twitter.com homepage should be filtered, falls through to GitHub meta
      expect(result?.docsUrl).toBe('https://autoprefixer.github.io')
    })

    it('skips homepage if its a package registry URL', async () => {
      const { fetchGitHubRepoMeta, fetchReadme } = await import('../../src/sources/github')
      const { fetchLlmsUrl } = await import('../../src/sources/llms')

      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          name: 'cross-env',
          version: '7.0.3',
          homepage: 'https://www.npmjs.com/package/cross-env',
          repository: { url: 'https://github.com/kentcdodds/cross-env' },
        }),
      })
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ time: { '7.0.3': '2024-01-01T00:00:00Z' } }),
      })

      vi.mocked(fetchGitHubRepoMeta).mockResolvedValue(null)
      vi.mocked(fetchReadme).mockResolvedValue('ungh://kentcdodds/cross-env')
      vi.mocked(fetchLlmsUrl).mockResolvedValue(null)

      const result = await resolvePackageDocs('cross-env')

      // npmjs.com homepage should be filtered, no docsUrl set
      expect(result?.docsUrl).toBeUndefined()
      expect(result?.readmeUrl).toBe('ungh://kentcdodds/cross-env')
    })

    it('skips homepage if its a GitHub URL', async () => {
      const { fetchGitHubRepoMeta, fetchReadme } = await import('../../src/sources/github')
      const { fetchLlmsUrl } = await import('../../src/sources/llms')

      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          name: 'pkg',
          version: '1.0.0',
          homepage: 'https://github.com/owner/repo',
          repository: { url: 'https://github.com/owner/repo' },
        }),
      })
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ time: { '1.0.0': '2024-01-01T00:00:00Z' } }),
      })

      vi.mocked(fetchGitHubRepoMeta).mockResolvedValue({ homepage: 'https://docs.example.com' })
      vi.mocked(fetchReadme).mockResolvedValue('ungh://owner/repo')
      vi.mocked(fetchLlmsUrl).mockResolvedValue(null)

      const result = await resolvePackageDocs('pkg')

      expect(result?.docsUrl).toBe('https://docs.example.com')
      expect(result?.readmeUrl).toBe('ungh://owner/repo')
    })

    it('falls back to README when no docs URL', async () => {
      const { fetchGitHubRepoMeta, fetchReadme } = await import('../../src/sources/github')
      const { fetchLlmsUrl } = await import('../../src/sources/llms')

      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          name: 'simple-pkg',
          version: '1.0.0',
          repository: { url: 'https://github.com/owner/repo' },
        }),
      })
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ time: { '1.0.0': '2024-01-01T00:00:00Z' } }),
      })

      vi.mocked(fetchGitHubRepoMeta).mockResolvedValue(null)
      vi.mocked(fetchReadme).mockResolvedValue('https://raw.githubusercontent.com/owner/repo/main/README.md')
      vi.mocked(fetchLlmsUrl).mockResolvedValue(null)

      const result = await resolvePackageDocs('simple-pkg')

      expect(result?.readmeUrl).toBe('https://raw.githubusercontent.com/owner/repo/main/README.md')
    })

    it('returns null when no docs sources found', async () => {
      const { fetchGitHubRepoMeta, fetchReadme } = await import('../../src/sources/github')
      const { fetchLlmsUrl } = await import('../../src/sources/llms')

      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          name: 'bare-pkg',
          version: '1.0.0',
          // No homepage, no repository
        }),
      })
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ time: { '1.0.0': '2024-01-01T00:00:00Z' } }),
      })

      vi.mocked(fetchGitHubRepoMeta).mockResolvedValue(null)
      vi.mocked(fetchReadme).mockResolvedValue(null)
      vi.mocked(fetchLlmsUrl).mockResolvedValue(null)

      const result = await resolvePackageDocs('bare-pkg')

      expect(result).toBeNull()
    })

    it('handles repository subdirectory', async () => {
      const { fetchGitHubRepoMeta, fetchReadme } = await import('../../src/sources/github')
      const { fetchLlmsUrl } = await import('../../src/sources/llms')

      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          name: '@nuxt/kit',
          version: '3.10.0',
          repository: {
            url: 'https://github.com/nuxt/nuxt',
            directory: 'packages/kit',
          },
        }),
      })
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ time: { '3.10.0': '2024-01-01T00:00:00Z' } }),
      })

      vi.mocked(fetchGitHubRepoMeta).mockResolvedValue(null)
      vi.mocked(fetchReadme).mockResolvedValue('ungh://nuxt/nuxt/packages/kit')
      vi.mocked(fetchLlmsUrl).mockResolvedValue(null)

      const result = await resolvePackageDocs('@nuxt/kit')

      expect(result?.readmeUrl).toBe('ungh://nuxt/nuxt/packages/kit')
      expect(fetchReadme).toHaveBeenCalledWith('nuxt', 'nuxt', 'packages/kit', undefined)
    })
  })

  describe('fetchPkgDist', () => {
    let mockFileStream: any

    beforeEach(() => {
      vi.resetAllMocks()
      const closeListeners: Array<() => void> = []
      mockFileStream = {
        write: vi.fn((_chunk: any, cb: () => void) => cb()),
        end: vi.fn(),
        destroy: vi.fn(() => {
          for (const cb of closeListeners) cb()
        }),
        on: vi.fn().mockReturnThis(),
        once: vi.fn((event: string, cb: () => void) => {
          if (event === 'close')
            closeListeners.push(cb)
          return mockFileStream
        }),
      }
    })

    afterEach(() => {
      vi.restoreAllMocks()
      vi.unstubAllGlobals()
    })

    it('cleans up stream and temp file on download failure', async () => {
      const { existsSync, mkdirSync, rmSync, createWriteStream } = await import('node:fs')
      vi.mocked(existsSync).mockReturnValue(false)
      vi.mocked(mkdirSync).mockReturnValue(undefined as any)
      vi.mocked(rmSync).mockReturnValue(undefined)
      vi.mocked(createWriteStream).mockReturnValue(mockFileStream)

      // Registry metadata
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          dist: { tarball: 'https://registry.npmjs.org/pkg/-/pkg-1.0.0.tgz' },
        }),
      })

      // Tarball download succeeds but body reader fails mid-transfer
      const mockCancel = vi.fn().mockResolvedValue(undefined)
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
        ok: true,
        body: {
          getReader: () => ({
            read: vi.fn().mockRejectedValue(new Error('network timeout')),
            cancel: mockCancel,
          }),
        },
      }))

      const result = await fetchPkgDist('pkg', '1.0.0')

      expect(result).toBeNull()
      expect(mockCancel).toHaveBeenCalled()
      expect(mockFileStream.destroy).toHaveBeenCalled()
      expect(vi.mocked(rmSync)).toHaveBeenCalledWith(
        expect.stringContaining('_pkg.tgz'),
        { force: true },
      )
    })

    it('cleans up on tar extraction failure', async () => {
      const { existsSync, mkdirSync, rmSync, createWriteStream } = await import('node:fs')
      const { spawnSync } = await import('node:child_process')

      vi.mocked(existsSync).mockReturnValue(false)
      vi.mocked(mkdirSync).mockReturnValue(undefined as any)
      vi.mocked(rmSync).mockReturnValue(undefined)
      vi.mocked(createWriteStream).mockReturnValue(mockFileStream)
      vi.mocked(spawnSync).mockReturnValue({ status: 1 } as any)

      // Registry metadata
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({
          dist: { tarball: 'https://registry.npmjs.org/pkg/-/pkg-1.0.0.tgz' },
        }),
      })

      // Simulate successful download - reader returns one chunk then done
      const mockCancel = vi.fn().mockResolvedValue(undefined)
      const mockReader = {
        read: vi.fn()
          .mockResolvedValueOnce({ done: false, value: Buffer.from('fake-tarball') })
          .mockResolvedValueOnce({ done: true, value: undefined }),
        cancel: mockCancel,
      }

      // Need fileStream.on('close') to fire after end() for promise to resolve
      mockFileStream.on = vi.fn((event: string, cb: () => void) => {
        if (event === 'close')
          setTimeout(cb, 0)
        return mockFileStream
      })

      vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
        ok: true,
        body: { getReader: () => mockReader },
      }))

      const result = await fetchPkgDist('pkg', '1.0.0')

      expect(result).toBeNull()
      expect(mockFileStream.destroy).toHaveBeenCalled()
      expect(vi.mocked(rmSync)).toHaveBeenCalledWith(
        expect.stringContaining('_pkg.tgz'),
        { force: true },
      )
      expect(vi.mocked(rmSync)).toHaveBeenCalledWith(
        expect.stringContaining('pkg'),
        expect.objectContaining({ recursive: true }),
      )
    })
  })
})
