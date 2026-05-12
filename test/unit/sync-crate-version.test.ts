import { beforeEach, describe, expect, it, vi } from 'vitest'

const stopAfterVersion = new Error('stop-after-version')
const isCachedMock = vi.fn((_: string, __: string) => {
  throw stopAfterVersion
})

vi.mock('../../src/cache/index.ts', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../src/cache/index')>()
  return {
    ...actual,
    createReferenceCache: (name: string, version: string) => ({
      ...actual.createReferenceCache(name, version),
      has: () => isCachedMock(name, version),
    }),
  }
})

vi.mock('../../src/sources/resolver.ts', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../src/sources/resolver')>()
  return {
    ...actual,
    resolvePackageDocsWithAttempts: vi.fn().mockResolvedValue({
      package: {
        name: 'tokio',
        version: '2.0.0',
        docsUrl: 'https://tokio.dev',
      },
      attempts: [
        {
          source: 'npm',
          status: 'success',
        },
      ],
    }),
  }
})

vi.mock('../../src/sources/local-package.ts', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../src/sources/local-package')>()
  return {
    ...actual,
    readLocalDependencies: vi.fn().mockResolvedValue([]),
  }
})

vi.mock('../../src/sources/crates.ts', () => ({
  resolveCrateDocsWithAttempts: vi.fn().mockResolvedValue({
    package: {
      name: 'serde',
      version: '1.0.220',
      docsUrl: 'https://docs.rs/serde/1.0.220',
    },
    attempts: [
      {
        source: 'crates',
        status: 'success',
      },
    ],
  }),
}))

const { syncCommand } = await import('../../src/commands/sync')

describe('commands/sync crate version selection', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('uses resolved crate version for cache checks when requested version falls back', async () => {
    await expect(syncCommand(
      {
        skills: [],
        deps: new Map(),
        missing: [],
        outdated: [],
        synced: [],
        unmatched: [],
        shipped: [],
      },
      {
        packages: ['crate:serde@1.0.200'],
        global: false,
        agent: 'claude-code',
        yes: true,
      },
    )).rejects.toThrow(stopAfterVersion)

    expect(isCachedMock).toHaveBeenCalledWith('@skilld-crate/serde', '1.0.220')
  })

  it('keeps npm package cache key unnamespaced for same-name package', async () => {
    await expect(syncCommand(
      {
        skills: [],
        deps: new Map(),
        missing: [],
        outdated: [],
        synced: [],
        unmatched: [],
        shipped: [],
      },
      {
        packages: ['tokio'],
        global: false,
        agent: 'claude-code',
        yes: true,
      },
    )).rejects.toThrow(stopAfterVersion)

    expect(isCachedMock).toHaveBeenCalledWith('tokio', '2.0.0')
  })
})
