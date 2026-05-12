import { agents } from '../agent/index.ts'

export const sharedArgs = {
  global: {
    type: 'boolean' as const,
    alias: 'g',
    description: 'Install globally to ~/<agent>/skills',
    default: false,
  },
  agent: {
    type: 'enum' as const,
    options: Object.keys(agents),
    alias: 'a',
    description: 'Target agent — where skills are installed',
  },
  model: {
    type: 'string' as const,
    alias: 'm',
    description: 'Enhancement model for SKILL.md generation',
    valueHint: 'id',
  },
  yes: {
    type: 'boolean' as const,
    alias: 'y',
    description: 'Skip prompts, use defaults',
    default: false,
  },
  force: {
    type: 'boolean' as const,
    alias: 'f',
    description: 'Ignore all caches, re-fetch docs and regenerate',
    default: false,
  },
  debug: {
    type: 'boolean' as const,
    description: 'Save raw enhancement output to logs/ for each section',
    default: false,
  },
}
