import { mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createSkillHarness } from '../../src/index.ts'
import { createFakeHarness, createFakeSandboxProvider, skillSource } from '../support/fakes.ts'

interface SkillNameFixture {
  valid: string[]
  invalid: string[]
  maximumLength: number
}

async function loadFixture(): Promise<SkillNameFixture> {
  const path = new URL('../../../../contracts/fixtures/skill-conformance/skill-name.json', import.meta.url)
  return JSON.parse(await readFile(path, 'utf8')) as SkillNameFixture
}

describe('skill name contract', () => {
  it('matches the shared Skill name fixture through the public run boundary', async () => {
    const fixture = await loadFixture()
    const projectDir = await mkdtemp(join(tmpdir(), 'skilld-name-source-'))
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-name-output-'))
    await writeFile(join(projectDir, 'package.json'), '{}\n')
    const valid = [...fixture.valid, 'a'.repeat(fixture.maximumLength)]

    for (const name of valid) {
      const fake = createFakeHarness({
        async onPrompt({ sandbox, workDir }) {
          await sandbox.writeTextFile({
            path: join(workDir, 'skilld-output', name, 'SKILL.md'),
            content: skillSource(name),
          })
        },
      })
      const result = await createSkillHarness({
        harness: fake.harness,
        sandbox: createFakeSandboxProvider(),
      }).run({
        _tag: 'ProjectSkill',
        projectDir,
        destination: { rootDir: destinationRoot, name },
      })
      expect(result._tag).toBe('Ok')
    }

    const invalid = [...fixture.invalid, 'a'.repeat(fixture.maximumLength + 1)]
    const fake = createFakeHarness({ onPrompt: async () => {} })
    const harness = createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider() })
    for (const name of invalid) {
      const result = await harness.run({
        _tag: 'ProjectSkill',
        projectDir,
        destination: { rootDir: destinationRoot, name },
      })
      expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'InvalidInput' } })
    }
    expect(fake.capture.starts).toHaveLength(0)
  })
})
