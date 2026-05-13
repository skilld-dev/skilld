import type { AgentType } from '../agent/index.ts'
import { styleText } from 'node:util'
import * as p from '@clack/prompts'
import { agents, detectInstalledAgents, detectProjectAgents, detectTargetAgent } from '../agent/index.ts'
import { readConfig, updateConfig } from '../core/config.ts'
import { isInteractive } from './env.ts'

export function resolveAgent(agentFlag?: string): AgentType | 'none' | null {
  if (process.env.SKILLD_NO_AGENT)
    return null
  return (agentFlag as AgentType | undefined)
    ?? detectTargetAgent()
    ?? (readConfig().agent as AgentType | undefined)
    ?? null
}

/**
 * Auto-resolve target agent without prompting. Consults, in order:
 *   1. Explicit `--agent` flag.
 *   2. Env-detected target (`CLAUDE_CODE`, `CURSOR_SESSION`, etc).
 *   3. Saved config default.
 *   4. Project marker dirs (`.claude/skills`, `.cursor/`, …) when exactly one
 *      agent matches the cwd.
 *   5. Installed agents on the machine when exactly one is found.
 *
 * Returns `null` when nothing is determined. Callers should error with a clear
 * "pass --agent <name>" hint rather than prompting — keeps commands
 * scriptable.
 */
export function autoResolveAgent(agentFlag?: string): AgentType | null {
  const resolved = resolveAgent(agentFlag)
  if (resolved && resolved !== 'none')
    return resolved

  if (process.env.SKILLD_NO_AGENT)
    return null

  const projectMatches = detectProjectAgents()
  if (projectMatches.length === 1)
    return projectMatches[0]!

  const installed = detectInstalledAgents()
  if (installed.length === 1)
    return installed[0]!

  return null
}

export function listKnownAgents(): AgentType[] {
  return Object.keys(agents) as AgentType[]
}

let _warnedNoAgent = false
function warnNoAgent(): void {
  if (_warnedNoAgent)
    return
  _warnedNoAgent = true
  p.log.warn('No target agent detected — falling back to prompt-only mode.\n  Use --agent <name> to specify, or run `skilld config` to set a default.')
}

export async function promptForAgent(): Promise<AgentType | 'none' | null> {
  const noAgent = !!process.env.SKILLD_NO_AGENT
  const installed = noAgent ? [] : detectInstalledAgents()
  const projectMatches = noAgent ? [] : detectProjectAgents()

  if (!isInteractive()) {
    if (installed.length === 1) {
      updateConfig({ agent: installed[0] })
      return installed[0]!
    }
    warnNoAgent()
    return 'none'
  }

  p.log.info(
    `Skilld generates reference cards from package docs so your AI agent\n`
    + `  always has accurate APIs for your exact dependency versions.`,
  )

  const candidateIds = projectMatches.length > 0
    ? projectMatches
    : installed.length > 0
      ? installed
      : Object.keys(agents) as AgentType[]

  const sharedAgents = new Set(
    Object.entries(agents)
      .filter(([, a]) => a.additionalSkillsDirs.some(d => d.includes('.claude/skills')))
      .map(([id]) => id),
  )

  const sharedIds = candidateIds.filter(id => id === 'claude-code' || sharedAgents.has(id))
  const isolatedIds = candidateIds.filter(id => id !== 'claude-code' && !sharedAgents.has(id))

  const options: Array<{ label: string, value: AgentType | 'none', hint?: string }> = []

  if (sharedIds.length > 0 && isolatedIds.length > 0) {
    for (const id of sharedIds) {
      const a = agents[id]
      const hint = id === 'claude-code'
        ? `skills shared with ${sharedIds.length - 1} other agents`
        : `skills shared with Claude Code and others`
      options.push({ label: a.displayName, value: id as AgentType, hint })
    }
  }

  const isolatedAgentIds = new Set(
    Object.entries(agents)
      .filter(([, a]) => a.additionalSkillsDirs.length === 0)
      .map(([id]) => id),
  )

  for (const id of (sharedIds.length > 0 && isolatedIds.length > 0 ? isolatedIds : candidateIds)) {
    if (options.some(o => o.value === id))
      continue
    const a = agents[id]
    const hint = sharedAgents.has(id) && id !== 'claude-code'
      ? 'skills shared with Claude Code and others'
      : isolatedAgentIds.has(id)
        ? 'skills only visible to this agent'
        : undefined
    options.push({ label: a.displayName, value: id as AgentType, hint })
  }

  options.push({ label: 'No agent', value: 'none', hint: 'export as standalone files for any AI' })

  if (!_warnedNoAgent) {
    _warnedNoAgent = true
    const hint = projectMatches.length > 1
      ? `Multiple agent directories found: ${projectMatches.map(t => agents[t].displayName).join(', ')}`
      : installed.length > 0
        ? `Found ${installed.map(t => agents[t].displayName).join(', ')} but couldn't determine which to use`
        : 'No agents auto-detected'
    const crossNote = sharedIds.length > 1
      ? `\n  ${styleText('gray', `Tip: Picking Claude Code shares skills with ${sharedIds.filter(id => id !== 'claude-code').map(id => agents[id].displayName).join(', ')} automatically.`)}`
      : ''
    p.log.warn(`${hint}\n  Pick the agent you actively code with.${crossNote}`)
  }

  const choice = await p.select({
    message: 'Which AI coding agent do you use?',
    options,
  })

  if (p.isCancel(choice))
    return null

  if (choice === 'none')
    return 'none'

  updateConfig({ agent: choice })
  p.log.success(`Target agent set to ${agents[choice].displayName}`)
  return choice
}
