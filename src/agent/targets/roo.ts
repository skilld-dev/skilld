import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'pathe'
import { defineTarget, SPEC_FRONTMATTER } from './base.ts'

const home = () => homedir()

/**
 * Roo Code (VS Code extension)
 *
 * IMPORTANT: Roo does NOT read .claude/skills/ or .agents/skills/.
 * It requires its own .roo/skills/ directory — no cross-compat shortcuts.
 *
 * Unique feature: mode-specific skill directories (.roo/skills-{modeSlug}/)
 * allow targeting skills to specific modes (code, architect, etc.).
 *
 * @see https://docs.roocode.com/features/skills
 * @see https://docs.roocode.com/features/custom-instructions
 */
export const roo = defineTarget({
  agent: 'roo',
  displayName: 'Roo Code',
  detectInstalled: () => existsSync(join(home(), '.roo')),
  detectEnv: () => !!process.env.ROO_SESSION,
  detectProject: cwd => existsSync(join(cwd, '.roo')),
  instructionFile: '.roorules',

  skillsDir: '.roo/skills',
  globalSkillsDir: () => join(home(), '.roo/skills'),

  frontmatter: [
    { ...SPEC_FRONTMATTER.name!, description: 'Must exactly match the directory name.' },
    { ...SPEC_FRONTMATTER.description!, description: 'When to activate.' },
  ],

  discoveryStrategy: 'eager',
  discoveryNotes: 'Reads all SKILL.md files at startup. File watchers detect changes during session. Uses read_file to load full content on activation.',

  agentSkillsSpec: false,
  extensions: [
    'Mode-specific skill directories: .roo/skills-{modeSlug}/',
  ],

  docs: 'https://docs.roocode.com/features/skills',
  notes: [
    'Does NOT read .claude/skills/ or .agents/skills/ — requires its own .roo/skills/ directory.',
    'Mode-specific dirs: .roo/skills-code/, .roo/skills-architect/ etc. target specific modes.',
    'Override priority: project mode-specific > project generic > global mode-specific > global generic.',
    'Supports symlinks for shared skill libraries across projects.',
    'Rules system (.roo/rules/) is separate — .md/.txt files loaded alphabetically into system prompt.',
    'Legacy fallback: .roorules file if .roo/rules/ is empty.',
    'Skills manageable from Settings panel (v3.46.0+).',
  ],
})
