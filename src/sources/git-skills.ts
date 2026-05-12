/**
 * Git repo skill source — parse inputs + fetch pre-authored skills from repos
 *
 * Supports GitHub shorthand (owner/repo), full URLs, SSH, GitLab, and local paths.
 * Skills are pre-authored SKILL.md files — no doc resolution or LLM generation needed.
 */

import { existsSync, readdirSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { downloadTemplate } from 'giget'
import { join, resolve } from 'pathe'
import { parseFrontmatter } from '../core/markdown.ts'
import { normalizeRepoUrl, parseGitHubUrl } from '../core/url.ts'
import { getGitHubToken } from './github-common.ts'
import { $fetch, fetchGitHubRaw } from './utils.ts'

export interface GitSkillSource {
  type: 'github' | 'gitlab' | 'git-ssh' | 'local'
  owner?: string
  repo?: string
  /** Direct path to a specific skill (from /tree/ref/path URLs) */
  skillPath?: string
  /** Branch/tag parsed from URL */
  ref?: string
  /** Absolute path for local sources */
  localPath?: string
}

export interface RemoteSkill {
  /** From SKILL.md frontmatter `name` field, or directory name */
  name: string
  /** From SKILL.md frontmatter `description` field */
  description: string
  /** Path within repo (e.g., "skills/web-design-guidelines") */
  path: string
  /** Full SKILL.md content */
  content: string
  /** Supporting files (scripts/, references/, assets/) */
  files: Array<{ path: string, content: string }>
}

/**
 * Detect whether an input string is a git skill source.
 * Returns null for npm package names (including scoped @scope/pkg).
 */
export function parseGitSkillInput(input: string): GitSkillSource | null {
  const trimmed = input.trim()

  // Scoped npm packages → not git
  if (trimmed.startsWith('@'))
    return null

  // Local paths
  if (trimmed.startsWith('./') || trimmed.startsWith('../') || trimmed.startsWith('/') || trimmed.startsWith('~')) {
    const localPath = trimmed.startsWith('~')
      ? resolve(process.env.HOME || '', trimmed.slice(1))
      : resolve(trimmed)
    return { type: 'local', localPath }
  }

  // SSH format: git@github.com:owner/repo
  if (trimmed.startsWith('git@')) {
    const normalized = normalizeRepoUrl(trimmed)
    const gh = parseGitHubUrl(normalized)
    if (gh)
      return { type: 'github', owner: gh.owner, repo: gh.repo }
    return null
  }

  // Full URLs
  if (trimmed.startsWith('https://') || trimmed.startsWith('http://')) {
    return parseGitUrl(trimmed)
  }

  // GitHub shorthand: owner/repo (exactly one slash, no spaces, no commas)
  if (/^[\w.-]+\/[\w.-]+$/.test(trimmed)) {
    return { type: 'github', owner: trimmed.split('/')[0], repo: trimmed.split('/')[1] }
  }

  // Everything else → npm
  return null
}

function parseGitUrl(url: string): GitSkillSource | null {
  try {
    const parsed = new URL(url)

    if (parsed.hostname === 'github.com' || parsed.hostname === 'www.github.com') {
      const parts = parsed.pathname.replace(/^\//, '').replace(/\.git$/, '').split('/')
      const owner = parts[0]
      const repo = parts[1]
      if (!owner || !repo)
        return null

      // Handle /tree/ref/path URLs → extract specific skill path
      if (parts[2] === 'tree' && parts.length >= 4) {
        const ref = parts[3]
        const skillPath = parts.length > 4 ? parts.slice(4).join('/') : undefined
        return { type: 'github', owner, repo, ref, skillPath }
      }

      return { type: 'github', owner, repo }
    }

    if (parsed.hostname === 'gitlab.com') {
      const parts = parsed.pathname.replace(/^\//, '').replace(/\.git$/, '').split('/')
      const owner = parts[0]
      const repo = parts[1]
      if (!owner || !repo)
        return null
      return { type: 'gitlab', owner, repo }
    }

    return null
  }
  catch {
    return null
  }
}

/**
 * Parse name and description from SKILL.md frontmatter.
 */
export function parseSkillFrontmatterName(content: string): { name?: string, description?: string } {
  const fm = parseFrontmatter(content)
  return { name: fm.name, description: fm.description }
}

/** Recursively find all directories containing a SKILL.md file. */
function findSkillDirs(root: string, prefix = ''): Array<{ dir: string, repoPath: string }> {
  const out: Array<{ dir: string, repoPath: string }> = []
  if (!existsSync(root))
    return out
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    if (!entry.isDirectory())
      continue
    const dir = resolve(root, entry.name)
    const repoPath = prefix ? `${prefix}/${entry.name}` : entry.name
    if (existsSync(resolve(dir, 'SKILL.md')))
      out.push({ dir, repoPath })
    else
      out.push(...findSkillDirs(dir, repoPath))
  }
  return out
}

/** Recursively collect all files in a directory, returning relative paths */
function collectFiles(dir: string, prefix = ''): Array<{ path: string, content: string }> {
  const files: Array<{ path: string, content: string }> = []
  if (!existsSync(dir))
    return files
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const relPath = prefix ? `${prefix}/${entry.name}` : entry.name
    const fullPath = resolve(dir, entry.name)
    if (entry.isDirectory()) {
      files.push(...collectFiles(fullPath, relPath))
    }
    else if (entry.isFile()) {
      files.push({ path: relPath, content: readFileSync(fullPath, 'utf-8') })
    }
  }
  return files
}

/**
 * Fetch skills from a git source. Returns list of discovered skills.
 */
export async function fetchGitSkills(
  source: GitSkillSource,
  onProgress?: (msg: string) => void,
): Promise<{ skills: RemoteSkill[] }> {
  if (source.type === 'local')
    return fetchLocalSkills(source)
  if (source.type === 'github')
    return fetchGitHubSkills(source, onProgress)
  if (source.type === 'gitlab')
    return fetchGitLabSkills(source, onProgress)
  return { skills: [] }
}

// ── Local ──

function fetchLocalSkills(source: GitSkillSource): { skills: RemoteSkill[] } {
  const base = source.localPath!
  if (!existsSync(base))
    return { skills: [] }

  const skills: RemoteSkill[] = []

  // Check for skills/ subdirectory (recursive — repos may nest by category)
  const skillsDir = resolve(base, 'skills')
  if (existsSync(skillsDir)) {
    for (const { dir, repoPath } of findSkillDirs(skillsDir, 'skills')) {
      const skill = readLocalSkill(dir, repoPath)
      if (skill)
        skills.push(skill)
    }
  }

  // Check for root SKILL.md
  if (skills.length === 0) {
    const skill = readLocalSkill(base, '')
    if (skill)
      skills.push(skill)
  }

  return { skills }
}

function readLocalSkill(dir: string, repoPath: string): RemoteSkill | null {
  const skillMdPath = resolve(dir, 'SKILL.md')
  if (!existsSync(skillMdPath))
    return null

  const content = readFileSync(skillMdPath, 'utf-8')
  const frontmatter = parseSkillFrontmatterName(content)
  const dirName = dir.split('/').pop()!
  const name = frontmatter.name || dirName

  // Collect all files except SKILL.md (handled separately)
  const files = collectFiles(dir).filter(f => f.path !== 'SKILL.md')

  return {
    name,
    description: frontmatter.description || '',
    path: repoPath,
    content,
    files,
  }
}

// ── GitHub ──

async function fetchGitHubSkills(
  source: GitSkillSource,
  onProgress?: (msg: string) => void,
): Promise<{ skills: RemoteSkill[] }> {
  const { owner, repo } = source
  if (!owner || !repo)
    return { skills: [] }

  const ref = source.ref || 'main'
  const refs = ref === 'main' ? ['main', 'master'] : [ref]

  for (const tryRef of refs) {
    const skills = await downloadGitHubSkills(owner, repo, tryRef, source.skillPath, onProgress)
    if (skills.length > 0)
      return { skills }
  }

  return { skills: [] }
}

async function downloadGitHubSkills(
  owner: string,
  repo: string,
  ref: string,
  skillPath?: string,
  onProgress?: (msg: string) => void,
): Promise<RemoteSkill[]> {
  const tempDir = join(tmpdir(), `skilld-${Date.now()}`)

  try {
    if (skillPath) {
      onProgress?.(`Downloading ${owner}/${repo}/${skillPath}@${ref}`)
      const { dir } = await downloadTemplate(
        `github:${owner}/${repo}/${skillPath}#${ref}`,
        { dir: tempDir, force: true, auth: getGitHubToken() || undefined },
      )
      const skill = readLocalSkill(dir, skillPath)
      return skill ? [skill] : []
    }

    // Download skills/ subdirectory (single tarball request)
    onProgress?.(`Downloading ${owner}/${repo}/skills@${ref}`)
    try {
      const { dir } = await downloadTemplate(
        `github:${owner}/${repo}/skills#${ref}`,
        { dir: tempDir, force: true, auth: getGitHubToken() || undefined },
      )

      const skills: RemoteSkill[] = []
      for (const { dir: skillDir, repoPath } of findSkillDirs(dir, 'skills')) {
        const skill = readLocalSkill(skillDir, repoPath)
        if (skill)
          skills.push(skill)
      }

      if (skills.length > 0) {
        onProgress?.(`Found ${skills.length} skill(s)`)
        return skills
      }
    }
    catch {}

    // Fallback: check root SKILL.md via single HTTP request (auth-aware for private repos)
    const content = await fetchGitHubRaw(
      `https://raw.githubusercontent.com/${owner}/${repo}/${ref}/SKILL.md`,
    )
    if (content) {
      const fm = parseSkillFrontmatterName(content)
      onProgress?.('Found 1 skill')
      return [{
        name: fm.name || repo,
        description: fm.description || '',
        path: '',
        content,
        files: [],
      }]
    }

    return []
  }
  catch {
    return []
  }
  finally {
    rmSync(tempDir, { recursive: true, force: true })
  }
}

// ── GitLab ──

async function fetchGitLabSkills(
  source: GitSkillSource,
  onProgress?: (msg: string) => void,
): Promise<{ skills: RemoteSkill[] }> {
  const { owner, repo } = source
  if (!owner || !repo)
    return { skills: [] }

  const ref = source.ref || 'main'
  const tempDir = join(tmpdir(), `skilld-gitlab-${Date.now()}`)

  try {
    const subdir = source.skillPath || 'skills'
    onProgress?.(`Downloading ${owner}/${repo}/${subdir}@${ref}`)

    const { dir } = await downloadTemplate(
      `gitlab:${owner}/${repo}/${subdir}#${ref}`,
      { dir: tempDir, force: true },
    )

    if (source.skillPath) {
      const skill = readLocalSkill(dir, source.skillPath)
      return { skills: skill ? [skill] : [] }
    }

    const skills: RemoteSkill[] = []
    for (const { dir: skillDir, repoPath } of findSkillDirs(dir, 'skills')) {
      const skill = readLocalSkill(skillDir, repoPath)
      if (skill)
        skills.push(skill)
    }

    if (skills.length > 0) {
      onProgress?.(`Found ${skills.length} skill(s)`)
      return { skills }
    }

    // Fallback: check root SKILL.md
    const content = await $fetch(
      `https://gitlab.com/${owner}/${repo}/-/raw/${ref}/SKILL.md`,
      { responseType: 'text' },
    ).catch(() => null)
    if (content) {
      const fm = parseSkillFrontmatterName(content)
      return {
        skills: [{
          name: fm.name || repo,
          description: fm.description || '',
          path: '',
          content,
          files: [],
        }],
      }
    }

    return { skills: [] }
  }
  catch {
    return { skills: [] }
  }
  finally {
    rmSync(tempDir, { recursive: true, force: true })
  }
}
