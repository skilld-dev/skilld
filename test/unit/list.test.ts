import { describe, expect, it, vi } from 'vitest'

const NOW = new Date().toISOString()

const mockSkills = [
  {
    name: 'vue-skilld',
    dir: '/skills/vue-skilld',
    agent: 'claude-code',
    info: { version: '3.4.0', source: 'github.com/vuejs/core', syncedAt: NOW, generator: 'skilld', packageName: 'vue' },
    scope: 'local',
  },
  {
    name: 'nuxt-skilld',
    dir: '/skills/nuxt-skilld',
    agent: 'claude-code',
    info: { version: '3.10.0', source: 'github.com/nuxt/nuxt', syncedAt: NOW, generator: 'skilld', packageName: 'nuxt' },
    scope: 'local',
  },
]

const mockOutdated = [
  {
    ...mockSkills[0],
    packageName: 'vue',
    latestVersion: '3.5.0',
  },
]

// Mock the agent layer to avoid @earendil-works/pi-ai import chain
vi.mock('../../src/agent/index.ts', () => ({
  agents: {},
  detectCurrentAgent: () => null,
  detectTargetAgent: () => 'claude-code',
  detectProjectAgents: () => [],
  getAgentVersion: () => null,
  getModelName: () => '',
}))

vi.mock('../../src/core/skills.ts', () => ({
  iterateSkills: vi.fn(function* () {
    for (const s of mockSkills) yield s
  }),
  getProjectState: vi.fn(async () => ({
    skills: mockSkills,
    deps: new Map([['vue', '3.5.0'], ['nuxt', '3.10.0']]),
    missing: [],
    outdated: mockOutdated,
    synced: [mockSkills[1]],
    unmatched: [],
  })),
}))

const { listCommand } = await import('../../src/commands/list.ts')

describe('listCommand', () => {
  it('outputs JSON with --json', async () => {
    const writeSpy = vi.spyOn(process.stdout, 'write').mockImplementation(() => true)
    await listCommand({ json: true })

    const output = writeSpy.mock.calls[0]![0] as string
    const parsed = JSON.parse(output)
    expect(parsed).toHaveLength(2)
    expect(parsed[0].name).toBe('vue-skilld')
    expect(parsed[1].name).toBe('nuxt-skilld')
    writeSpy.mockRestore()
  })

  it('outputs table format by default', async () => {
    const writeSpy = vi.spyOn(process.stdout, 'write').mockImplementation(() => true)
    await listCommand()

    expect(writeSpy).toHaveBeenCalledTimes(2)
    const firstLine = writeSpy.mock.calls[0]![0] as string
    expect(firstLine).toContain('vue-skilld')
    writeSpy.mockRestore()
  })

  it('shows only outdated skills with --outdated', async () => {
    const writeSpy = vi.spyOn(process.stdout, 'write').mockImplementation(() => true)
    await listCommand({ outdated: true })

    expect(writeSpy).toHaveBeenCalledTimes(1)
    const line = writeSpy.mock.calls[0]![0] as string
    expect(line).toContain('vue-skilld')
    expect(line).toContain('3.4.0')
    expect(line).toContain('3.5.0')
    expect(line).toContain('→')
    expect(line).not.toContain('nuxt-skilld')
    writeSpy.mockRestore()
  })

  it('shows "up to date" when no outdated skills with --outdated', async () => {
    const { getProjectState } = await import('../../src/core/skills.ts')
    vi.mocked(getProjectState).mockResolvedValueOnce({
      skills: [],
      deps: new Map(),
      missing: [],
      outdated: [],
      synced: [],
      unmatched: [],
    })

    const writeSpy = vi.spyOn(process.stdout, 'write').mockImplementation(() => true)
    await listCommand({ outdated: true })

    const output = writeSpy.mock.calls[0]![0] as string
    expect(output).toContain('up to date')
    writeSpy.mockRestore()
  })

  it('outputs outdated skills as JSON with --outdated --json', async () => {
    const writeSpy = vi.spyOn(process.stdout, 'write').mockImplementation(() => true)
    await listCommand({ outdated: true, json: true })

    const output = writeSpy.mock.calls[0]![0] as string
    const parsed = JSON.parse(output)
    expect(parsed).toHaveLength(1)
    expect(parsed[0].name).toBe('vue-skilld')
    expect(parsed[0].version).toBe('3.4.0')
    expect(parsed[0].latest).toBe('3.5.0')
    writeSpy.mockRestore()
  })
})
