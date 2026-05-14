/**
 * Project layout: every `~/.skilld/...` and `.claude/skills/...` path lives here.
 *
 * Two layers:
 *   1. Global cache under `~/.skilld/` (references, repos, llm-cache, etc.)
 *   2. Per-skill internals: `<skillDir>/.skilld/` holds reference symlinks,
 *      logs, and the canonical `_SKILL.md`. The lockfile sits as a sibling
 *      of skill dirs as `skilld-lock.yaml`.
 *
 * Per-agent target dirs (`.claude/skills/`, `.cursor/skills/`, ...) are owned
 * by the agent registry (`src/agent/registry.ts`); this module exposes the
 * agent-agnostic `.skills/` shared dir and helpers that compose with target dirs.
 */

import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'pathe'

// ── Project-relative names ──

/** Shared, agent-agnostic skills directory at project root */
export const SHARED_SKILLS_DIR = '.skills'

/** Per-skill internals directory (cache symlinks, logs, prompts, _SKILL.md) */
export const SKILL_INTERNAL_DIRNAME = '.skilld'

/** Canonical SKILL.md copy inside the per-skill internals dir (frontmatter source of truth) */
export const SKILL_INTERNAL_FILENAME = '_SKILL.md'

/** Lockfile sibling of skill directories */
export const LOCK_FILENAME = 'skilld-lock.yaml'

/** Config filename inside the global cache */
export const CONFIG_FILENAME = 'config.yaml'

// ── Global cache layout (~/.skilld/) ──

/** Root of the global cache */
export const CACHE_DIR: string = join(homedir(), '.skilld')

/** References subdirectory: `~/.skilld/references/<pkg>@<version>/` */
export const REFERENCES_DIR: string = join(CACHE_DIR, 'references')

/** Repo-level cache (issues, discussions, releases shared across monorepo packages) */
export const REPOS_DIR: string = join(CACHE_DIR, 'repos')

/** LLM output cache: `~/.skilld/llm-cache/<sha256>.json` */
export const LLM_CACHE_DIR: string = join(CACHE_DIR, 'llm-cache')

/** Global config file */
export const CONFIG_PATH: string = join(CACHE_DIR, CONFIG_FILENAME)

/** pi-ai auth credentials */
export const PI_AI_AUTH_PATH: string = join(CACHE_DIR, 'pi-ai-auth.json')

/** CLI auth marker (`~/.skilld/auth.json`, 0600). Stores tokens directly only when keychain unavailable. */
export const AUTH_PATH: string = join(CACHE_DIR, 'auth.json')

// ── Helpers ──

/** Returns the shared skills directory path if `.skills/` exists at project root, else null */
export function getSharedSkillsDir(cwd: string = process.cwd()): string | null {
  const dir = join(cwd, SHARED_SKILLS_DIR)
  return existsSync(dir) ? dir : null
}

/** Path to the lockfile inside a skills directory */
export function lockfilePath(skillsDir: string): string {
  return join(skillsDir, LOCK_FILENAME)
}

/** Per-skill internals dir (`<skillDir>/.skilld`) */
export function skillInternalDir(skillDir: string): string {
  return join(skillDir, SKILL_INTERNAL_DIRNAME)
}

/** Per-skill canonical SKILL.md (`<skillDir>/.skilld/_SKILL.md`) */
export function skillInternalFile(skillDir: string): string {
  return join(skillDir, SKILL_INTERNAL_DIRNAME, SKILL_INTERNAL_FILENAME)
}

/** Per-skill log dir (`<skillDir>/.skilld/logs`) */
export function skillLogDir(skillDir: string): string {
  return join(skillDir, SKILL_INTERNAL_DIRNAME, 'logs')
}

/** Per-skill section dir (`<skillDir>/.skilld/<section>`), e.g. issues/discussions/releases/docs */
export function skillRefsSection(skillDir: string, section: string): string {
  return join(skillDir, SKILL_INTERNAL_DIRNAME, section)
}

/** References cache dir for a `name@version` */
export function getReferencesDir(name: string, version: string): string {
  return join(REFERENCES_DIR, `${name}@${version}`)
}

/** Repo cache dir with path-traversal validation */
export function getRepoCacheDir(owner: string, repo: string): string {
  if (owner.includes('..') || repo.includes('..') || owner.includes('/') || repo.includes('/'))
    throw new Error(`Invalid repo path: ${owner}/${repo}`)
  return join(REPOS_DIR, owner, repo)
}

/** search.db path for a `name@version` */
export function getPackageDbPath(name: string, version: string): string {
  return join(getReferencesDir(name, version), 'search.db')
}
