import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { autoResolveAgent } from '../../src/cli/agent-prompt.ts'

const detection = vi.hoisted(() => ({
  configAgent: undefined as string | undefined,
  envAgent: null as string | null,
  installedAgents: [] as string[],
  projectAgents: [] as string[],
}))

vi.mock('../../src/agent/index.ts', () => ({
  agents: {},
  detectEnvAgent: () => detection.envAgent,
  detectInstalledAgents: () => detection.installedAgents,
  detectProjectAgents: () => detection.projectAgents,
  detectTargetAgent: () => detection.envAgent
    ?? (detection.projectAgents.length === 1 ? detection.projectAgents[0] : null),
}))

vi.mock('../../src/core/config.ts', () => ({
  readConfig: () => ({ agent: detection.configAgent }),
  updateConfig: vi.fn(),
}))

describe('autoResolveAgent', () => {
  const originalNoAgent = process.env.SKILLD_NO_AGENT

  beforeEach(() => {
    detection.configAgent = undefined
    detection.envAgent = null
    detection.installedAgents = []
    detection.projectAgents = []
    delete process.env.SKILLD_NO_AGENT
  })

  afterEach(() => {
    if (originalNoAgent === undefined)
      delete process.env.SKILLD_NO_AGENT
    else
      process.env.SKILLD_NO_AGENT = originalNoAgent
  })

  it('prefers the saved config over a single project marker', () => {
    detection.configAgent = 'claude-code'
    detection.projectAgents = ['cursor']

    expect(autoResolveAgent()).toBe('claude-code')
  })
})
