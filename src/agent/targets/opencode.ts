import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'pathe'
import { defineTarget, SPEC_FRONTMATTER } from './base.ts'

const configHome = () => process.env.XDG_CONFIG_HOME || join(homedir(), '.config')

/**
 * OpenCode (SST)
 *
 * Walks from CWD up to git worktree root searching for skill dirs.
 * Reads .claude/skills/ and .agents/skills/ in addition to .opencode/skills/.
 *
 * Has a rich agent system: .opencode/agents/ with per-agent model/tool configuration.
 * Skills can be permission-controlled per-agent with allow/deny/ask + glob patterns.
 *
 * @see https://opencode.ai/docs/skills/
 * @see https://opencode.ai/docs/rules/
 */
export const opencode = defineTarget({
  agent: 'opencode',
  displayName: 'OpenCode',
  detectInstalled: () => existsSync(join(configHome(), 'opencode')),
  detectEnv: () => !!(process.env.OPENCODE_SESSION || process.env.OPENCODE_SESSION_ID),
  detectProject: cwd => existsSync(join(cwd, '.opencode')),
  instructionFile: 'AGENTS.md',

  skillsDir: '.opencode/skills',
  globalSkillsDir: () => join(configHome(), 'opencode/skills'),
  additionalSkillsDirs: [
    '.claude/skills',
    '.agents/skills',
    '~/.claude/skills',
    '~/.agents/skills',
  ],

  frontmatter: [
    { ...SPEC_FRONTMATTER.name!, description: 'Must match directory name.' },
    { ...SPEC_FRONTMATTER.description!, description: 'Used for matching.' },
    SPEC_FRONTMATTER.license!,
    SPEC_FRONTMATTER.compatibility!,
    SPEC_FRONTMATTER.metadata!,
  ],

  discoveryStrategy: 'eager',
  discoveryNotes: 'Walks from CWD to git worktree root, then loads global definitions. Agents access skills via native skill tool. Skills can be permission-controlled per-agent.',

  agentSkillsSpec: true,
  extensions: [
    'Per-agent skill permissions (allow/deny/ask with glob patterns)',
  ],

  docs: 'https://opencode.ai/docs/skills/',
  notes: [
    'Reads .claude/skills/ and .agents/skills/ natively — emitting to .claude/skills/ covers multiple agents.',
    'Custom agents in .opencode/agents/ have rich config: model, temperature, tools, permission, color.',
    'opencode.json supports an instructions field with glob patterns pointing to instruction files.',
    'AGENTS.md (or CLAUDE.md fallback) for general instructions.',
  ],
})
