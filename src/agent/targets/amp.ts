import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'pathe'
import { defineTarget, SPEC_FRONTMATTER } from './base.ts'

const configHome = () => process.env.XDG_CONFIG_HOME || join(homedir(), '.config')

/**
 * Amp (Sourcegraph)
 *
 * Uses .agents/skills/ as primary project path. Also reads .claude/skills/.
 * Skills can bundle MCP servers via mcp.json in the skill directory.
 *
 * AGENTS.md (or AGENT.md / CLAUDE.md fallback) for general instructions,
 * supports @-mentions to reference other files and glob-based conditional includes.
 *
 * @see https://ampcode.com/news/agent-skills
 * @see https://ampcode.com/manual
 */
export const amp = defineTarget({
  agent: 'amp',
  displayName: 'Amp',
  detectInstalled: () => existsSync(join(configHome(), 'amp')),
  detectEnv: () => !!process.env.AMP_SESSION,
  detectProject: cwd => existsSync(join(cwd, '.agents', 'AGENTS.md')),
  instructionFile: 'AGENTS.md',

  skillsDir: '.agents/skills',
  globalSkillsDir: () => join(configHome(), 'agents/skills'),
  additionalSkillsDirs: [
    '.claude/skills',
    '~/.config/amp/skills',
    '~/.claude/skills',
  ],

  frontmatter: [
    { ...SPEC_FRONTMATTER.name!, description: 'Unique identifier. Project skills override user-wide ones with same name.' },
    { ...SPEC_FRONTMATTER.description!, description: 'Always visible to the model; determines when skill is invoked.' },
  ],

  discoveryStrategy: 'lazy',
  discoveryNotes: 'Names + descriptions visible at startup. Full SKILL.md body loads only when agent decides to invoke based on description match.',

  agentSkillsSpec: false,
  extensions: [
    'mcp.json for bundling MCP server configurations',
  ],

  docs: 'https://ampcode.com/news/agent-skills',
  notes: [
    'Reads .claude/skills/ natively — emitting there covers Claude Code, Cursor, Cline, Copilot, AND Amp.',
    'Skills can bundle MCP servers via mcp.json in the skill directory.',
    'AGENTS.md supports @-mentions to reference files (e.g. @doc/style.md, @doc/*.md globs).',
    'AGENTS.md files with globs frontmatter are conditionally included only when Amp reads matching files.',
  ],
})
