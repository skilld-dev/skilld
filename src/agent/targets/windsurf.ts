import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'pathe'
import { defineTarget, SPEC_FRONTMATTER } from './base.ts'

const home = () => homedir()

/**
 * Windsurf (Codeium editor)
 *
 * Has TWO systems: Rules (.windsurf/rules/*.md) and Skills (.windsurf/skills/).
 * We target Skills. Rules have a separate frontmatter schema with trigger/globs.
 *
 * Skills only document `name` and `description` as frontmatter fields.
 * Cascade uses "progressive disclosure" for supporting files.
 *
 * @see https://docs.windsurf.com/windsurf/cascade/skills
 * @see https://docs.windsurf.com/windsurf/cascade/memories
 */
export const windsurf = defineTarget({
  agent: 'windsurf',
  displayName: 'Windsurf',
  detectInstalled: () => existsSync(join(home(), '.codeium/windsurf')),
  detectEnv: () => !!process.env.WINDSURF_SESSION,
  detectProject: cwd => existsSync(join(cwd, '.windsurf')) || existsSync(join(cwd, '.windsurfrules')),
  instructionFile: '.windsurfrules',

  skillsDir: '.windsurf/skills',
  globalSkillsDir: () => join(home(), '.codeium/windsurf/skills'),

  frontmatter: [
    { ...SPEC_FRONTMATTER.name!, description: 'Skill identifier.', constraints: 'Lowercase, numbers, hyphens only' },
    { ...SPEC_FRONTMATTER.description!, description: 'Used by Cascade for automatic invocation matching.' },
  ],

  discoveryStrategy: 'eager',
  discoveryNotes: 'Cascade matches description against user requests for auto-invocation. Manual invocation via @skill-name.',

  agentSkillsSpec: false,

  docs: 'https://docs.windsurf.com/windsurf/cascade/skills',
  notes: [
    'Only `name` and `description` are documented as frontmatter fields. Other fields may be silently ignored.',
    'Rules system is separate: .windsurf/rules/*.md with trigger/globs/alwaysApply frontmatter.',
    'Rules have a 6,000 char per-file limit and 12,000 char total limit. Skills have no documented limit.',
    'Legacy .windsurfrules at project root still supported but deprecated.',
    'Supporting files alongside SKILL.md are loaded via progressive disclosure.',
  ],
})
