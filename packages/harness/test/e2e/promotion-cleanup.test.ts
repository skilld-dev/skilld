import type { PathLike, RmOptions } from 'node:fs'
import type * as FileSystem from 'node:fs/promises'
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, vi } from 'vitest'
import { createSkillHarness } from '../../src/index.ts'
import { createFakeHarness, createFakeSandboxProvider, skillSource } from '../support/fakes.ts'

const failures = vi.hoisted(() => ({ backup: false, lock: false, promotion: false }))

vi.mock('node:fs/promises', async (importOriginal) => {
  const fileSystem = await importOriginal<typeof FileSystem>()
  return {
    ...fileSystem,
    async rm(path: PathLike, options?: RmOptions) {
      if (failures.backup && String(path).endsWith('.previous'))
        throw new Error('backup cleanup blocked')
      return fileSystem.rm(path, options)
    },
    async unlink(path: PathLike) {
      if (failures.lock && String(path).endsWith('.lock'))
        throw new Error('lock cleanup blocked')
      return fileSystem.unlink(path)
    },
    async rename(oldPath: PathLike, newPath: PathLike) {
      if (failures.promotion && String(oldPath).endsWith('.next'))
        throw new Error('promotion blocked')
      return fileSystem.rename(oldPath, newPath)
    },
  }
})

async function runReplacement() {
  const projectDir = await mkdtemp(join(tmpdir(), 'skilld-project-'))
  const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
  const currentDir = join(destinationRoot, 'example-project')
  await writeFile(join(projectDir, 'package.json'), '{}\n')
  await mkdir(currentDir)
  await writeFile(join(currentDir, 'SKILL.md'), skillSource('example-project', '# Old\n'))
  const fake = createFakeHarness({
    async onPrompt({ sandbox, workDir }) {
      await sandbox.writeTextFile({
        path: join(workDir, 'skilld-output/example-project/SKILL.md'),
        content: skillSource('example-project', '# New\n'),
      })
    },
  })
  const result = await createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider() }).run({
    _tag: 'ProjectSkill',
    projectDir,
    destination: { rootDir: destinationRoot, name: 'example-project' },
  })
  return { currentDir, destinationRoot, result }
}

describe('promotion cleanup', () => {
  afterEach(() => {
    failures.backup = false
    failures.lock = false
    failures.promotion = false
  })

  it('reports backup cleanup after committing the replacement', async () => {
    failures.backup = true

    const { currentDir, result } = await runReplacement()

    expect(result).toMatchObject({
      _tag: 'Ok',
      value: { warnings: [expect.stringContaining('Previous Skill')] },
    })
    await expect(readFile(join(currentDir, 'SKILL.md'), 'utf8')).resolves.toContain('# New')
  })

  it('reports lock cleanup after committing the replacement', async () => {
    failures.lock = true

    const { currentDir, destinationRoot, result } = await runReplacement()

    expect(result).toMatchObject({
      _tag: 'Ok',
      value: { warnings: [expect.stringContaining('Output lock')] },
    })
    await expect(readFile(join(currentDir, 'SKILL.md'), 'utf8')).resolves.toContain('# New')
    await expect(readFile(join(destinationRoot, '.skilld-example-project.lock'), 'utf8')).resolves.toBe('')
  })

  it('restores the current Skill when replacement promotion fails', async () => {
    failures.promotion = true

    const { currentDir, result } = await runReplacement()

    expect(result).toMatchObject({
      _tag: 'Err',
      error: { _tag: 'PromotionFailed', message: 'Skill output could not be promoted.' },
    })
    await expect(readFile(join(currentDir, 'SKILL.md'), 'utf8')).resolves.toContain('# Old')
  })
})
