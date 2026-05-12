import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'pathe'
import { defineTarget, SPEC_FRONTMATTER } from './base.ts'

const home = () => homedir()

/**
 * Cline (VS Code extension)
 *
 * Has TWO systems: Rules (.clinerules/) and Skills (.cline/skills/).
 * We target Skills. Cline also reads .claude/skills/ as a fallback,
 * so emitting to .claude/skills/ covers both Claude Code and Cline.
 *
 * Only `name` and `description` are parsed from frontmatter.
 * All other fields are stripped/ignored.
 *
 * @see https://docs.cline.bot/features/skills
 * @see https://docs.cline.bot/features/cline-rules
 */
export const cline = defineTarget({
  agent: 'cline',
  displayName: 'Cline',
  detectInstalled: () => existsSync(join(home(), '.cline')),
  detectEnv: () => !!(process.env.CLINE_TASK_ID || process.env.CLINE_ACTIVE),
  detectProject: cwd => existsSync(join(cwd, '.cline')),
  instructionFile: '.clinerules',

  skillsDir: '.cline/skills',
  globalSkillsDir: () => join(home(), '.cline/skills'),
  additionalSkillsDirs: [
    '.clinerules/skills',
    '.claude/skills',
  ],

  frontmatter: [
    { ...SPEC_FRONTMATTER.name!, description: 'Must exactly match the directory name.' },
    { ...SPEC_FRONTMATTER.description!, description: 'When to activate. Used for matching.' },
  ],

  discoveryStrategy: 'eager',
  discoveryNotes: 'At startup reads name + description from each skill. Full content loads on-demand via use_skill tool. Dozens of skills have near-zero context cost.',

  agentSkillsSpec: false,

  docs: 'https://docs.cline.bot/features/skills',
  notes: [
    'Only `name` and `description` are parsed. `version`, `globs`, etc. are silently ignored.',
    'Cline reads .claude/skills/ as a fallback — emitting there covers both Claude Code and Cline.',
    'Rules system (.clinerules/) is separate: always-on behavioral constraints with globs/tags frontmatter.',
    'Global skills override project skills when names conflict.',
    'Supporting dirs: docs/, scripts/, templates/ alongside SKILL.md.',
  ],
})
