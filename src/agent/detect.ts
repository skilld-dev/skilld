/**
 * Agent detection - identify installed and active agents
 */

import type { AgentType } from './types.ts'
import { spawnSync } from 'node:child_process'
import { isWindows } from 'std-env'
import { agents } from './targets/index.ts'

const STATIC_REGEX_1 = /v?(\d+\.\d+\.\d+(?:-[a-z0-9.]+)?)/

/**
 * Detect which agents are installed on the system
 */
export function detectInstalledAgents(): AgentType[] {
  return Object.entries(agents)
    .filter(([_, config]) => config.detectInstalled())
    .map(([type]) => type as AgentType)
}

/** Detect the active agent from environment variables. */
export function detectEnvAgent(): AgentType | null {
  for (const [type, target] of Object.entries(agents)) {
    if (target.detectEnv())
      return type as AgentType
  }
  return null
}

/**
 * Detect the target agent (where skills are installed) from env vars and cwd.
 * This is NOT the generator LLM — it determines the skills directory.
 *
 * Priority: env vars first (running inside agent = unambiguous), then project dirs.
 * When multiple agents match project dirs, returns null to trigger user prompt
 * rather than silently picking the first match.
 */
export function detectTargetAgent(): AgentType | null {
  const envAgent = detectEnvAgent()
  if (envAgent)
    return envAgent

  const cwd = process.cwd()
  const projectMatches: AgentType[] = []
  for (const [type, target] of Object.entries(agents)) {
    if (target.detectProject(cwd))
      projectMatches.push(type as AgentType)
  }

  // Single match is unambiguous; multiple matches need user disambiguation
  return projectMatches.length === 1 ? projectMatches[0]! : null
}

/**
 * Get all agents matching the current project directory.
 * Used by promptForAgent to show relevant agents first when disambiguation is needed.
 */
export function detectProjectAgents(): AgentType[] {
  const cwd = process.cwd()
  return Object.entries(agents)
    .filter(([, target]) => target.detectProject(cwd))
    .map(([type]) => type as AgentType)
}

/**
 * Get the version of an agent's CLI (if available)
 */
export function getAgentVersion(agentType: AgentType): string | null {
  const agent = agents[agentType]
  if (!agent.cli)
    return null

  try {
    const result = spawnSync(agent.cli, ['--version'], {
      encoding: 'utf-8',
      timeout: 3000,
      stdio: ['pipe', 'pipe', 'pipe'],
      shell: isWindows,
    })
    if (result.status !== 0)
      return null
    const output = (result.stdout || '').trim()

    // Extract version number from output
    // Common formats: "v1.2.3", "1.2.3", "cli 1.2.3", "name v1.2.3"
    const match = output.match(STATIC_REGEX_1)
    return match ? match[1]! : output.split('\n')[0]!
  }
  catch {
    return null
  }
}
