import type { Document, IndexConfig } from '../../src/retriv/index.ts'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'pathe'
import { afterEach, describe, expect, it } from 'vitest'
import { findPackageDbs } from '../../src/commands/search-helpers.ts'
import { createSelfSkill, MAX_SELF_FILE_BYTES, scanSelfProject } from '../../src/commands/self.ts'
import { selfIndexDbPath } from '../../src/core/paths.ts'
import { MAX_INDEX_DOCS } from '../../src/retriv/index-pipeline.ts'

const fixtures: string[] = []
const originalCwd = process.cwd()

function makeProject(): string {
  const cwd = mkdtempSync(join(tmpdir(), 'skilld-self-'))
  fixtures.push(cwd)
  writeFileSync(join(cwd, 'package.json'), JSON.stringify({
    name: '@demo/project',
    description: 'Demo project',
    main: 'src/index.ts',
  }))
  mkdirSync(join(cwd, 'src'))
  writeFileSync(join(cwd, 'src/index.ts'), 'export const answer = 42\n')
  writeFileSync(join(cwd, 'README.md'), '# Demo\n')
  return cwd
}

afterEach(() => {
  process.chdir(originalCwd)
  for (const fixture of fixtures.splice(0))
    rmSync(fixture, { recursive: true, force: true })
})

describe('scanSelfProject', () => {
  it('indexes project text without following symlinks or oversized files', async () => {
    const cwd = makeProject()
    mkdirSync(join(cwd, 'docs'))
    const outside = join(tmpdir(), `skilld-self-secret-${Date.now()}.md`)
    writeFileSync(outside, 'secret outside project')
    fixtures.push(outside)
    symlinkSync(outside, join(cwd, 'docs/linked.md'))
    writeFileSync(join(cwd, 'docs/oversized.md'), Buffer.alloc(MAX_SELF_FILE_BYTES + 1, 97))

    const project = await scanSelfProject(cwd)

    expect(project.files).toContain('README.md')
    expect(project.files).toContain('src/index.ts')
    expect(project.files).not.toContain('docs/linked.md')
    expect(project.files).not.toContain('docs/oversized.md')
    expect(project.documents.some(doc => doc.content.includes('secret outside project'))).toBe(false)
  })

  it('caps the project before reading source files', async () => {
    const cwd = makeProject()
    for (let i = 0; i < MAX_INDEX_DOCS; i++)
      writeFileSync(join(cwd, `source-${String(i).padStart(3, '0')}.ts`), `export const n = ${i}\n`)

    const project = await scanSelfProject(cwd)

    expect(project.documents).toHaveLength(MAX_INDEX_DOCS)
    expect(project.files[0]).toBe('README.md')
  })
})

describe('createSelfSkill', () => {
  it('rebuilds edited files and exposes the self database to search', async () => {
    const cwd = makeProject()
    const dbPath = selfIndexDbPath(cwd)
    mkdirSync(join(cwd, '.skilld/self'), { recursive: true })
    writeFileSync(dbPath, 'stale')
    const indexed: Document[][] = []

    const index = async (documents: Document[], config: IndexConfig & { removeIds?: string[] }) => {
      expect(existsSync(config.dbPath)).toBe(false)
      expect(readFileSync(dbPath, 'utf8')).toBe(indexed.length === 0 ? 'stale' : `build-${indexed.length}`)
      indexed.push(documents)
      writeFileSync(config.dbPath, `build-${indexed.length}`)
    }

    const first = await createSelfSkill({ cwd, agent: 'codex', index })
    writeFileSync(join(cwd, 'src/index.ts'), 'export const answer = 43\n')
    const second = await createSelfSkill({ cwd, agent: 'codex', index })

    expect(indexed).toHaveLength(2)
    expect(indexed[1]?.find(doc => doc.id === 'src/index.ts')?.content).toContain('43')
    expect(indexed[1]?.find(doc => doc.id === 'src/index.ts')?.metadata?.package).toBe('demo-project-project')
    expect(indexed[1]?.find(doc => doc.id === 'src/index.ts')?.metadata?.referenceRoot).toBe('.agents/skills/demo-project-project/.skilld')
    expect(readFileSync(dbPath, 'utf8')).toBe('build-2')
    expect(readFileSync(join(first.skillDir, 'SKILL.md'), 'utf8')).toContain('skilld search "query" -p self')
    expect(second.skillName).toBe('demo-project-project')
    expect(realpathSync(join(first.skillDir, '.skilld/project'))).toBe(realpathSync(cwd))

    process.chdir(cwd)
    expect(findPackageDbs('self')).toEqual([dbPath])
    expect(findPackageDbs()).toEqual([dbPath])
    expect(findPackageDbs('vue')).toEqual([])
  })

  it('keeps the last usable index when rebuilding fails', async () => {
    const cwd = makeProject()
    const dbPath = selfIndexDbPath(cwd)
    mkdirSync(join(cwd, '.skilld/self'), { recursive: true })
    writeFileSync(dbPath, 'working')

    await expect(createSelfSkill({
      cwd,
      agent: 'codex',
      index: async (_documents, config) => {
        writeFileSync(config.dbPath, 'partial')
        throw new Error('embedding failed')
      },
    })).rejects.toThrow('embedding failed')

    expect(readFileSync(dbPath, 'utf8')).toBe('working')
    expect(existsSync(`${dbPath}.next`)).toBe(false)
  })

  it('restores the previous index when promotion fails', async () => {
    const cwd = makeProject()
    const dbPath = selfIndexDbPath(cwd)
    mkdirSync(join(cwd, '.skilld/self'), { recursive: true })
    writeFileSync(dbPath, 'working')

    await expect(createSelfSkill({
      cwd,
      agent: 'codex',
      index: async (_documents, config) => {
        writeFileSync(config.dbPath, 'complete')
      },
      renameFile: () => {
        throw new Error('rename failed')
      },
    })).rejects.toThrow('rename failed')

    expect(readFileSync(dbPath, 'utf8')).toBe('working')
    expect(existsSync(`${dbPath}.previous`)).toBe(false)
  })

  it('does not replace the index when the project reference is not replaceable', async () => {
    const cwd = makeProject()
    const dbPath = selfIndexDbPath(cwd)
    mkdirSync(join(cwd, '.skilld/self'), { recursive: true })
    writeFileSync(dbPath, 'working')
    const projectLink = join(cwd, '.agents/skills/demo-project-project/.skilld/project')
    mkdirSync(join(projectLink, '..'), { recursive: true })
    writeFileSync(projectLink, 'not a symlink')

    await expect(createSelfSkill({
      cwd,
      agent: 'codex',
      index: async (_documents, config) => {
        writeFileSync(config.dbPath, 'complete')
      },
    })).rejects.toThrow('Cannot replace non-symlink project reference')

    expect(readFileSync(dbPath, 'utf8')).toBe('working')
  })

  it('rejects a symlinked skill destination', async () => {
    const cwd = makeProject()
    const outside = mkdtempSync(join(tmpdir(), 'skilld-self-outside-'))
    fixtures.push(outside)
    const skillsDir = join(cwd, '.agents/skills')
    mkdirSync(skillsDir, { recursive: true })
    symlinkSync(outside, join(skillsDir, 'demo-project-project'), 'dir')

    await expect(createSelfSkill({
      cwd,
      agent: 'codex',
      index: async (_documents, config) => {
        writeFileSync(config.dbPath, 'complete')
      },
    })).rejects.toThrow('Refusing to write through symlink')

    expect(existsSync(join(outside, 'SKILL.md'))).toBe(false)
    expect(existsSync(join(outside, '_SKILL.md'))).toBe(false)
  })

  it('rejects a symlinked internal skill directory', async () => {
    const cwd = makeProject()
    const outside = mkdtempSync(join(tmpdir(), 'skilld-self-internal-outside-'))
    fixtures.push(outside)
    const skillDir = join(cwd, '.agents/skills/demo-project-project')
    mkdirSync(skillDir, { recursive: true })
    symlinkSync(outside, join(skillDir, '.skilld'), 'dir')

    await expect(createSelfSkill({
      cwd,
      agent: 'codex',
      index: async (_documents, config) => {
        writeFileSync(config.dbPath, 'complete')
      },
    })).rejects.toThrow('Refusing to write through symlink')

    expect(existsSync(join(outside, '_SKILL.md'))).toBe(false)
    expect(existsSync(join(outside, 'project'))).toBe(false)
  })
})
