import type { AgentType } from '../agent/index.ts'
import type { Document, IndexConfig } from '../retriv/index.ts'
import { copyFileSync, existsSync, lstatSync, mkdirSync, renameSync, rmSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs'
import { glob, lstat, readFile } from 'node:fs/promises'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { basename, dirname, join, relative } from 'pathe'
import { agents, linkSkillToAgents, sanitizeName } from '../agent/index.ts'
import { ensureProjectFiles } from '../agent/skill-installer.ts'
import { resolveAgent } from '../cli/agent-prompt.ts'
import { timedSpinner } from '../core/formatting.ts'
import { readPackageJsonSafe } from '../core/package-json.ts'
import { getSharedSkillsDir, selfIndexDbPath, skillInternalDir } from '../core/paths.ts'
import { yamlEscape } from '../core/yaml.ts'
import { MAX_INDEX_DOCS } from '../retriv/index-pipeline.ts'
import { createIndex, SearchDepsUnavailableError } from '../retriv/index.ts'
import { shutdownWorker } from '../retriv/pool.ts'

const PROJECT_PATTERNS = [
  'README*',
  'package.json',
  'docs/**/*.{md,mdx,txt}',
  '**/*.{ts,tsx,mts,cts,js,jsx,mjs,cjs,vue,svelte,astro,py,rs,go,java,kt,kts,rb,php,swift,cs,css,scss,html}',
]

const DOC_FILE_RE = /^(?:README|docs\/)|\.(?:md|mdx|txt)$/i
const KEY_FILE_RE = /^(?:README[^/]*|package\.json)$/i
const INDEX_FILE_RE = /(?:^|\/)index\.(?:[cm]?[jt]sx?|vue|svelte|astro)$/

const IGNORED_DIRS: Record<string, true> = {
  '.git': true,
  '.skilld': true,
  '.nuxt': true,
  '.output': true,
  '.next': true,
  'build': true,
  'coverage': true,
  'dist': true,
  'node_modules': true,
  'target': true,
}

export const MAX_SELF_FILE_BYTES = 512 * 1024

function isMissingPathError(error: unknown): boolean {
  return error instanceof Error && 'code' in error && error.code === 'ENOENT'
}

function assertSafeSkillDestination(cwd: string, skillDir: string): void {
  const relativeDir = relative(cwd, skillDir)
  if (relativeDir === '..' || relativeDir.startsWith('../'))
    throw new Error(`Skill destination escapes the project: ${skillDir}`)

  let current = cwd
  for (const segment of relativeDir.split('/')) {
    current = join(current, segment)
    try {
      if (lstatSync(current).isSymbolicLink())
        throw new Error(`Refusing to write through symlink: ${current}`)
    }
    catch (error) {
      if (!isMissingPathError(error))
        throw error
    }
  }
}

function assertReplaceableProjectLink(projectLink: string): boolean {
  try {
    if (!lstatSync(projectLink).isSymbolicLink())
      throw new Error(`Cannot replace non-symlink project reference: ${projectLink}`)
    return true
  }
  catch (error) {
    if (isMissingPathError(error))
      return false
    throw error
  }
}

export interface SelfProject {
  name: string
  description?: string
  files: string[]
  documents: Document[]
  keyFiles: string[]
  directories: string[]
}

export interface CreateSelfSkillOptions {
  cwd: string
  agent: AgentType
  onProgress?: (message: string) => void
  index?: (documents: Document[], config: IndexConfig & { removeIds?: string[] }) => Promise<void>
  renameFile?: (from: string, to: string) => void
}

export interface CreateSelfSkillResult {
  dbPath: string
  skillDir: string
  skillName: string
  documentCount: number
}

/** Scan the current project into documents suitable for the search index. */
export async function scanSelfProject(cwd: string): Promise<SelfProject> {
  const packageJson = readPackageJsonSafe(join(cwd, 'package.json'))?.parsed
  const rawName = typeof packageJson?.name === 'string' ? packageJson.name : basename(cwd)
  const description = typeof packageJson?.description === 'string' ? packageJson.description : undefined
  const files: string[] = []

  for await (const file of glob(PROJECT_PATTERNS, {
    cwd,
    exclude: (path: string) => path.split('/').some(segment => IGNORED_DIRS[segment]),
  })) {
    files.push(file)
  }

  files.sort((a, b) => {
    const aDoc = DOC_FILE_RE.test(a)
    const bDoc = DOC_FILE_RE.test(b)
    return Number(bDoc) - Number(aDoc) || a.localeCompare(b)
  })
  const uniqueFiles = [...new Set(files)]
  const indexableFiles: string[] = []
  for (const file of uniqueFiles) {
    if (indexableFiles.length === MAX_INDEX_DOCS)
      break
    const info = await lstat(join(cwd, file))
    if (info.isFile() && info.size <= MAX_SELF_FILE_BYTES)
      indexableFiles.push(file)
  }

  const documents = await Promise.all(indexableFiles.map(async (file) => {
    const content = await readFile(join(cwd, file), 'utf8')
    const type = DOC_FILE_RE.test(file) ? 'doc' : 'source'
    return {
      id: file,
      content,
      metadata: { package: rawName, source: `project/${file}`, type },
    } satisfies Document
  }))

  const entryFields = ['main', 'module', 'types']
  const declaredEntries = entryFields.flatMap((field) => {
    const value = packageJson?.[field]
    return typeof value === 'string' ? [value] : []
  })
  const keyFiles = [...new Set([
    ...indexableFiles.filter(file => KEY_FILE_RE.test(file)),
    ...declaredEntries.filter(file => indexableFiles.includes(file)),
    ...indexableFiles.filter(file => INDEX_FILE_RE.test(file)).slice(0, 8),
  ])]
  const directories = [...new Set(indexableFiles.map(file => dirname(file)).filter(dir => dir !== '.'))].slice(0, 12)

  return { name: rawName, description, files: indexableFiles, documents, keyFiles, directories }
}

/** Render the project skill that directs agents to the local self index. */
export function renderSelfSkill(project: SelfProject, skillName: string): string {
  const description = project.description
    ? `Project context for ${project.name}: ${project.description}`
    : `Project context for ${project.name}`
  const keyFiles = project.keyFiles.length > 0
    ? project.keyFiles.map(file => `- \`${file}\``).join('\n')
    : '- No conventional entry files detected'
  const directories = project.directories.length > 0
    ? project.directories.map(dir => `- \`${dir}/\``).join('\n')
    : '- Project root only'

  return `---\nname: ${skillName}\ndescription: ${yamlEscape(description)}\n---\n\n# ${project.name}\n\nUse this skill for questions and changes specific to this project.\n\n## Search\n\nRun \`skilld search "query" -p self\` to search the current project source and docs. Re-run \`skilld self\` after the project changes to rebuild the index.\n\n## Key files\n\n${keyFiles}\n\n## Directories\n\n${directories}\n`
}

/** Build a fresh project-local index and install its generated skill. */
export async function createSelfSkill(opts: CreateSelfSkillOptions): Promise<CreateSelfSkillResult> {
  const project = await scanSelfProject(opts.cwd)
  if (project.documents.length === 0)
    throw new Error('No project source or documentation files found')

  const skillName = `${sanitizeName(project.name)}-project`
  const shared = getSharedSkillsDir(opts.cwd)
  const baseDir = shared || join(opts.cwd, agents[opts.agent].skillsDir)
  const skillDir = join(baseDir, skillName)
  const internalDir = skillInternalDir(skillDir)
  const projectLink = join(internalDir, 'project')
  assertSafeSkillDestination(opts.cwd, internalDir)
  assertReplaceableProjectLink(projectLink)
  const dbPath = selfIndexDbPath(opts.cwd)
  const referenceRoot = `${relative(opts.cwd, skillDir)}/.skilld`
  for (const document of project.documents)
    document.metadata = { ...document.metadata, package: skillName, referenceRoot }
  const nextDbPath = `${dbPath}.next`
  const index = opts.index ?? createIndex
  const renameFile = opts.renameFile ?? renameSync

  mkdirSync(dirname(dbPath), { recursive: true })
  for (const suffix of ['', '-shm', '-wal'])
    rmSync(`${nextDbPath}${suffix}`, { force: true })

  try {
    await index(project.documents, {
      dbPath: nextDbPath,
      onProgress: ({ phase, current, total }) => opts.onProgress?.(`${phase} (${current}/${total})`),
    })
  }
  catch (error) {
    for (const suffix of ['', '-shm', '-wal'])
      rmSync(`${nextDbPath}${suffix}`, { force: true })
    throw error
  }

  if (!existsSync(nextDbPath))
    throw new Error('Search index build did not produce a database')

  const backupDbPath = `${dbPath}.previous`
  try {
    for (const suffix of ['', '-shm', '-wal']) {
      rmSync(`${backupDbPath}${suffix}`, { force: true })
      if (existsSync(`${dbPath}${suffix}`))
        copyFileSync(`${dbPath}${suffix}`, `${backupDbPath}${suffix}`)
    }
  }
  catch (error) {
    for (const suffix of ['', '-shm', '-wal'])
      rmSync(`${backupDbPath}${suffix}`, { force: true })
    throw error
  }

  try {
    for (const suffix of ['', '-shm', '-wal'])
      rmSync(`${dbPath}${suffix}`, { force: true })
    for (const suffix of ['', '-shm', '-wal']) {
      if (existsSync(`${nextDbPath}${suffix}`))
        renameFile(`${nextDbPath}${suffix}`, `${dbPath}${suffix}`)
    }
  }
  catch (error) {
    for (const suffix of ['', '-shm', '-wal']) {
      rmSync(`${dbPath}${suffix}`, { force: true })
      if (existsSync(`${backupDbPath}${suffix}`))
        copyFileSync(`${backupDbPath}${suffix}`, `${dbPath}${suffix}`)
      rmSync(`${nextDbPath}${suffix}`, { force: true })
    }
    throw error
  }
  finally {
    for (const suffix of ['', '-shm', '-wal'])
      rmSync(`${backupDbPath}${suffix}`, { force: true })
  }

  mkdirSync(internalDir, { recursive: true })
  if (assertReplaceableProjectLink(projectLink))
    unlinkSync(projectLink)
  symlinkSync(relative(internalDir, opts.cwd), projectLink, 'dir')
  const skillContent = renderSelfSkill(project, skillName)
  writeFileSync(join(skillDir, 'SKILL.md'), skillContent)
  writeFileSync(join(skillInternalDir(skillDir), '_SKILL.md'), skillContent)

  if (shared)
    linkSkillToAgents(skillName, shared, opts.cwd, opts.agent)

  return { dbPath, skillDir, skillName, documentCount: project.documents.length }
}

export const selfCommandDef = defineCommand({
  meta: { name: 'self', description: 'Build a searchable skill from the current project' },
  args: {
    agent: {
      type: 'enum' as const,
      options: Object.keys(agents),
      alias: 'a',
      description: 'Target agent',
    },
  },
  async run({ args }) {
    const cwd = process.cwd()
    const agent = resolveAgent(args.agent)
    if (!agent || agent === 'none')
      return

    const spin = timedSpinner()
    spin.start('Scanning project')
    try {
      const result = await createSelfSkill({
        cwd,
        agent,
        onProgress: message => spin.message(message),
      })
      await ensureProjectFiles({ cwd, agent, global: false })
      spin.stop(`Indexed ${result.documentCount} files`)
      p.outro(`Self skill written to ${relative(cwd, result.skillDir)}`)
    }
    catch (error) {
      spin.stop('Self indexing failed')
      if (error instanceof SearchDepsUnavailableError)
        p.log.error('Search requires the optional native dependencies')
      else
        p.log.error(error instanceof Error ? error.message : String(error))
      process.exitCode = 1
    }
    finally {
      await shutdownWorker()
    }
  },
})
