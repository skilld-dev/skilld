import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'pathe'
import { defineTarget, SPEC_FRONTMATTER } from './base.ts'

const claudeHome = () => process.env.CLAUDE_CONFIG_DIR || join(homedir(), '.claude')

/**
 * Claude Code (Anthropic CLI)
 *
 * Follows the Agent Skills open standard (agentskills.io) plus Claude-specific
 * extensions like `disable-model-invocation`, `user-invocable`, `context`, etc.
 *
 * Skills are discovered at startup — only `name` + `description` are read initially.
 * Full SKILL.md body loads when the agent invokes the skill (via Skill tool or auto-match).
 *
 * @see https://code.claude.com/docs/en/skills
 * @see https://agentskills.io/specification
 */
export const claudeCode = defineTarget({
  agent: 'claude-code',
  displayName: 'Claude Code',
  detectInstalled: () => existsSync(claudeHome()),
  detectEnv: () => !!(process.env.CLAUDE_CODE || process.env.CLAUDECODE || process.env.CLAUDE_CODE_ENTRYPOINT || process.env.CLAUDE_CONFIG_DIR),
  detectProject: (cwd) => {
    // Strong signals: actual Claude Code user config or skills usage
    if (existsSync(join(cwd, '.claude', 'settings.json')))
      return true
    if (existsSync(join(cwd, '.claude', 'settings.local.json')))
      return true
    if (existsSync(join(cwd, '.claude', 'skills')))
      return true
    // Medium signal: bare .claude/ dir (real users who haven't configured skills yet)
    // Note: CLAUDE.md alone is NOT checked - too many false positives from repo conventions
    return existsSync(join(cwd, '.claude'))
  },
  cli: 'claude',
  instructionFile: 'CLAUDE.md',

  skillsDir: '.claude/skills',
  globalSkillsDir: () => join(claudeHome(), 'skills'),

  frontmatter: [
    { ...SPEC_FRONTMATTER.name!, required: false, description: 'Skill identifier, becomes /slash-command. Defaults to directory name if omitted.', constraints: '1-64 chars, ^[a-z0-9]+(-[a-z0-9]+)*$' },
    { ...SPEC_FRONTMATTER.description!, description: 'What the skill does and when to use it. Used for auto-discovery matching.' },
    SPEC_FRONTMATTER.license!,
    SPEC_FRONTMATTER.compatibility!,
    SPEC_FRONTMATTER.metadata!,
    SPEC_FRONTMATTER['allowed-tools']!,
    { name: 'disable-model-invocation', required: false, description: 'When true, skill only loads via explicit /name invocation' },
    { name: 'user-invocable', required: false, description: 'When false, hides from / menu but still auto-loads' },
    { name: 'argument-hint', required: false, description: 'Hint shown during autocomplete, e.g. [issue-number]' },
    { name: 'model', required: false, description: 'Model to use when skill is active' },
    { name: 'context', required: false, description: 'Set to "fork" to run in a forked subagent context' },
    { name: 'agent', required: false, description: 'Subagent type when context: fork (e.g. Explore, Plan)' },
  ],

  discoveryStrategy: 'eager',
  discoveryNotes: 'Scans skill dirs at startup, reads name + description only. Full body loads on invocation. Budget: 2% of context window for all skill descriptions.',

  agentSkillsSpec: true,
  extensions: [
    'disable-model-invocation',
    'user-invocable',
    'argument-hint',
    'model',
    'context',
    'agent',
    'hooks',
    '$ARGUMENTS substitution',
    '!`command` dynamic context',
  ],

  docs: 'https://code.claude.com/docs/en/skills',
  notes: [
    '`globs` is NOT a valid frontmatter field for skills (only for rules). Unknown fields are silently ignored.',
    '`version` and `generated_by` should go under `metadata` map, not as top-level fields.',
    'Skill descriptions have a char budget of 2% of context window (~16k chars fallback). Override with SLASH_COMMAND_TOOL_CHAR_BUDGET env var.',
    'Keep SKILL.md under 500 lines. Move detailed reference to separate files.',
    'Supports monorepo auto-discovery: nested .claude/skills/ dirs in subdirectories.',
    'Supporting dirs: scripts/, references/, assets/ alongside SKILL.md.',
    'Project detection uses weighted signals: .claude/settings.json, .claude/settings.local.json, .claude/skills/ are strong; bare .claude/ is medium; CLAUDE.md alone is not checked (too many false positives from repo conventions).',
  ],
})
