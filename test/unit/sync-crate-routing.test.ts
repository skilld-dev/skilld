import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('../../src/commands/sync-parallel', () => ({
  syncPackagesParallel: vi.fn(),
}))

vi.mock('../../src/sources/crates.ts', () => ({
  resolveCrateDocsWithAttempts: vi.fn().mockResolvedValue({
    package: null,
    attempts: [
      {
        source: 'crates',
        status: 'not-found',
        message: 'Crate not found on crates.io',
      },
    ],
  }),
}))

const { syncCommand } = await import('../../src/commands/sync')
const { isCrateSpec } = await import('../../src/core/prefix')

describe('commands/sync crate routing', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('detects crate spec prefix', () => {
    expect(isCrateSpec('crate:serde')).toBe(true)
    expect(isCrateSpec('serde')).toBe(false)
    expect(isCrateSpec('crate')).toBe(false)
  })

  it('keeps npm batch on parallel path when mixed with crate specs', async () => {
    const { syncPackagesParallel } = await import('../../src/commands/sync-parallel')

    await syncCommand(
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
        packages: ['vue', 'nuxt', 'crate:'],
        global: false,
        agent: 'claude-code',
        yes: true,
      },
    )

    expect(vi.mocked(syncPackagesParallel)).toHaveBeenCalledTimes(1)
    expect(vi.mocked(syncPackagesParallel)).toHaveBeenCalledWith(
      expect.objectContaining({
        packages: ['vue', 'nuxt'],
      }),
    )
  })

  it('routes valid crate spec through single-package path while keeping npm parallel', async () => {
    const { syncPackagesParallel } = await import('../../src/commands/sync-parallel')
    const { log } = await import('@clack/prompts')
    const errorSpy = vi.spyOn(log, 'error').mockImplementation(() => {})

    await syncCommand(
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
        packages: ['vue', 'nuxt', 'crate:serde'],
        global: false,
        agent: 'claude-code',
        yes: true,
      },
    )

    expect(vi.mocked(syncPackagesParallel)).toHaveBeenCalledTimes(1)
    expect(vi.mocked(syncPackagesParallel)).toHaveBeenCalledWith(
      expect.objectContaining({
        packages: ['vue', 'nuxt'],
      }),
    )
    expect(errorSpy).not.toHaveBeenCalled()
  })

  it('does not invoke npm parallel sync when only crate specs are provided', async () => {
    const { syncPackagesParallel } = await import('../../src/commands/sync-parallel')
    const { log } = await import('@clack/prompts')
    const errorSpy = vi.spyOn(log, 'error').mockImplementation(() => {})

    await syncCommand(
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
        packages: ['crate:', 'crate:   '],
        global: false,
        agent: 'claude-code',
        yes: true,
      },
    )

    expect(vi.mocked(syncPackagesParallel)).not.toHaveBeenCalled()
    expect(errorSpy).toHaveBeenCalledTimes(2)
    expect(errorSpy).toHaveBeenNthCalledWith(1, 'Invalid crate spec. Use format: crate:<name>')
    expect(errorSpy).toHaveBeenNthCalledWith(2, 'Invalid crate spec. Use format: crate:<name>')
  })
})
