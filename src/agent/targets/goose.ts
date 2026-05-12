import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'pathe'
import { defineTarget, SPEC_FRONTMATTER } from './base.ts'

const configHome = () => process.env.XDG_CONFIG_HOME || join(homedir(), '.config')

/**
 * Goose (Block)
 *
 * Scans 6 directories for skills, including .claude/skills/ and .agents/skills/
 * for cross-agent compatibility. Later directories override earlier ones on
 * name conflict.
 *
 * @see https://block.github.io/goose/docs/guides/context-engineering/using-skills/
 */
export const goose = defineTarget({
  agent: 'goose',
  displayName: 'Goose',
  detectInstalled: () => existsSync(join(configHome(), 'goose')),
  detectEnv: () => !!(process.env.GOOSE_SESSION || process.env.AGENT_SESSION_ID),
  detectProject: cwd => existsSync(join(cwd, '.goose')),
  cli: 'goose',
  instructionFile: '.goosehints',

  skillsDir: '.goose/skills',
  globalSkillsDir: () => join(configHome(), 'goose/skills'),
  additionalSkillsDirs: [
    '.claude/skills',
    '.agents/skills',
    '~/.claude/skills',
    '~/.config/agents/skills',
  ],

  frontmatter: [
    { ...SPEC_FRONTMATTER.name!, description: 'Skill identifier.' },
    { ...SPEC_FRONTMATTER.description!, description: 'Brief purpose statement; used for matching.' },
  ],

  discoveryStrategy: 'eager',
  discoveryNotes: 'Scans all 6 directories at startup, merges discovered skills. Later directories override earlier ones on name conflict.',

  agentSkillsSpec: false,

  docs: 'https://block.github.io/goose/docs/guides/context-engineering/using-skills/',
  notes: [
    'Reads .claude/skills/ natively — emitting there covers both Claude Code and Goose.',
    'Also supports .goosehints / .goosehints.local for general project instructions (separate from skills).',
    'Supporting files alongside SKILL.md (scripts, templates, configs) are accessible.',
  ],
})
