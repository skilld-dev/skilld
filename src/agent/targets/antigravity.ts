import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'pathe'
import { defineTarget, SPEC_FRONTMATTER } from './base.ts'

const home = () => homedir()

/**
 * Antigravity (Google)
 *
 * Agent-first IDE (VS Code fork) powered by Gemini. Skills live in
 * .agent/skills/ (workspace) or ~/.gemini/antigravity/skills/ (global).
 * Uses semantic matching on description to auto-invoke skills.
 *
 * Adopted the Agent Skills open standard (agentskills.io) in Jan 2026.
 * Only `name` and `description` are used for routing; body loads on demand.
 *
 * @see https://antigravity.google/docs/skills
 * @see https://codelabs.developers.google.com/getting-started-with-antigravity-skills
 */
export const antigravity = defineTarget({
  agent: 'antigravity',
  displayName: 'Antigravity',
  detectInstalled: () => existsSync(join(home(), '.gemini/antigravity')),
  detectEnv: () => !!process.env.ANTIGRAVITY_CLI_ALIAS,
  detectProject: cwd => existsSync(join(cwd, '.agent')),
  instructionFile: 'GEMINI.md',

  skillsDir: '.agent/skills',
  globalSkillsDir: () => join(home(), '.gemini/antigravity/skills'),

  frontmatter: [
    { ...SPEC_FRONTMATTER.name!, description: 'Skill identifier. Defaults to directory name if omitted.' },
    { ...SPEC_FRONTMATTER.description!, description: 'Semantic trigger for agent routing. Must be descriptive enough for LLM matching.' },
  ],

  discoveryStrategy: 'lazy',
  discoveryNotes: 'Indexes name + description at startup. Full SKILL.md body loads on demand when agent semantic-matches description against user prompt.',

  agentSkillsSpec: true,

  docs: 'https://antigravity.google/docs/skills',
  notes: [
    'Only `name` and `description` are used for routing; other frontmatter fields are accepted but not used for matching.',
    'Skill directories can include scripts/, resources/, and assets/ subdirectories for supporting files.',
    'GEMINI.md instruction file is shared with Gemini CLI. .agent/rules/*.md for always-on workspace rules.',
  ],
})
