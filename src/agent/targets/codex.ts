import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'pathe'
import { defineTarget, SPEC_FRONTMATTER } from './base.ts'

const codexHome = () => process.env.CODEX_HOME || join(homedir(), '.codex')

/**
 * OpenAI Codex CLI
 *
 * IMPORTANT: Codex uses `.agents/skills/` for project-level skills,
 * NOT `.codex/skills/`. The `.codex/` directory is for config (config.toml).
 * `~/.codex/skills/` works only as a legacy user-global path.
 *
 * Codex also has AGENTS.md (or AGENTS.override.md) for general instructions,
 * which walks from git root to CWD concatenating found files.
 *
 * @see https://developers.openai.com/codex/skills
 * @see https://developers.openai.com/codex/guides/agents-md/
 */
export const codex = defineTarget({
  agent: 'codex',
  displayName: 'Codex',
  detectInstalled: () => existsSync(codexHome()),
  // CODEX_HOME is a config dir override, not a session indicator.
  // Codex doesn't currently set a reliable "running inside codex" env var,
  // so env detection is disabled - we rely on project dir detection instead.
  detectEnv: () => false,
  detectProject: cwd =>
    existsSync(join(cwd, '.codex'))
    || existsSync(join(cwd, 'AGENTS.md'))
    || existsSync(join(cwd, 'AGENTS.override.md'))
    || existsSync(join(cwd, '.agents', 'skills')),
  cli: 'codex',
  instructionFile: 'AGENTS.md',

  skillsDir: '.agents/skills',
  globalSkillsDir: () => join(homedir(), '.agents/skills'),
  additionalSkillsDirs: [
    '~/.codex/skills',
    '/etc/codex/skills',
  ],

  frontmatter: [
    { ...SPEC_FRONTMATTER.name!, description: 'Skill identifier.', constraints: '1-64 chars, ^[a-z0-9-]+$, no leading/trailing/consecutive hyphens' },
    { ...SPEC_FRONTMATTER.description!, description: 'Must include when-to-use criteria. Primary triggering mechanism.', constraints: '1-1024 chars, no angle brackets (< or >)' },
    SPEC_FRONTMATTER.license!,
    SPEC_FRONTMATTER['allowed-tools']!,
    SPEC_FRONTMATTER.metadata!,
  ],

  discoveryStrategy: 'lazy',
  discoveryNotes: 'Startup scan reads name + description + optional agents/openai.yaml. Full body loads only on invocation. Supports $1-$9 and $ARGUMENTS placeholders.',

  agentSkillsSpec: true,
  extensions: [
    'agents/openai.yaml (UI metadata + MCP dependencies)',
    '$1-$9 positional argument placeholders',
    'AGENTS.override.md for temporary overrides',
  ],

  skillActivationHint: 'Before modifying code, check .agents/skills/ for relevant skills.\nRead the SKILL.md for any matching package before proceeding.',

  docs: 'https://developers.openai.com/codex/skills',
  notes: [
    'Description field cannot contain angle brackets (< or >).',
    'Optional agents/openai.yaml provides UI metadata: display_name, icon, brand_color, default_prompt.',
    'AGENTS.md walks from git root to CWD, concatenating all found files.',
    'Live reload: detects skill file changes without restart (v0.95.0+).',
    'Size limit: 32 KiB default (project_doc_max_bytes), configurable in ~/.codex/config.toml.',
  ],
})
