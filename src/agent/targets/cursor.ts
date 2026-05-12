import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'pathe'
import { defineTarget, SPEC_FRONTMATTER } from './base.ts'

const home = () => homedir()

/**
 * Cursor (AI code editor)
 *
 * Has TWO systems: Rules (.cursor/rules/*.mdc) and Skills (.cursor/skills/).
 * We target the Skills system which follows the Agent Skills spec.
 *
 * Cursor natively scans .claude/skills/ and .codex/skills/ in addition to
 * its own .cursor/skills/ — so .claude/skills/ output works for both
 * Claude Code and Cursor with zero duplication.
 *
 * @see https://cursor.com/docs/context/skills
 * @see https://cursor.com/docs/context/rules
 */
export const cursor = defineTarget({
  agent: 'cursor',
  displayName: 'Cursor',
  detectInstalled: () => existsSync(join(home(), '.cursor')),
  detectEnv: () => !!(process.env.CURSOR_SESSION || process.env.CURSOR_TRACE_ID),
  detectProject: cwd => existsSync(join(cwd, '.cursor')) || existsSync(join(cwd, '.cursorrules')),
  instructionFile: '.cursor/rules/skilld-activation.mdc',

  skillsDir: '.cursor/skills',
  globalSkillsDir: () => join(home(), '.cursor/skills'),
  additionalSkillsDirs: [
    '.claude/skills',
    '.codex/skills',
    '~/.claude/skills',
    '~/.codex/skills',
  ],

  frontmatter: [
    SPEC_FRONTMATTER.name!,
    { ...SPEC_FRONTMATTER.description!, description: 'Agent uses this to decide relevance for auto-invocation.' },
    SPEC_FRONTMATTER.license!,
    SPEC_FRONTMATTER.compatibility!,
    SPEC_FRONTMATTER.metadata!,
    { name: 'disable-model-invocation', required: false, description: 'When true, only loads via explicit /skill-name' },
  ],

  discoveryStrategy: 'lazy',
  discoveryNotes: 'Reads name + description at conversation start. Full SKILL.md body loads only when agent determines relevance. Users can also invoke via /skill-name.',

  agentSkillsSpec: true,
  extensions: [
    'disable-model-invocation',
  ],

  skillActivationHint: 'Before modifying code, evaluate each installed skill against the current task.\nFor each skill, determine YES/NO relevance and invoke all YES skills before proceeding.',

  docs: 'https://cursor.com/docs/context/skills',
  notes: [
    'Cursor scans .claude/skills/ and .codex/skills/ natively — emitting to .claude/skills/ covers both Claude Code and Cursor.',
    'The Rules system (.cursor/rules/*.mdc) is separate and uses different frontmatter (trigger, globs, alwaysApply).',
    'Skills appear in Settings > Rules > Agent Decides section.',
    'Supporting dirs: scripts/, references/, assets/ alongside SKILL.md.',
  ],
})
