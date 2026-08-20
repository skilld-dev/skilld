import { mkdir, mkdtemp, readFile, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createSkillHarness } from '../../src/index.ts'
import { createFakeHarness, createFakeSandboxProvider, skillSource } from '../support/fakes.ts'

async function makePackage() {
  const root = await mkdtemp(join(tmpdir(), 'skilld-package-'))
  await writeFile(join(root, 'package.json'), '{"name":"example-package","exports":"./index.js"}\n')
  await writeFile(join(root, 'README.md'), '# Example package\n')
  return root
}

const promptText = (prompt: unknown): string => typeof prompt === 'string' ? prompt : JSON.stringify(prompt)

describe('createSkillHarness', () => {
  it('runs one visible package workflow and promotes its checked output', async () => {
    const packageDir = await makePackage()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    let configured = false
    const { harness, capture } = createFakeHarness({
      async onPrompt({ options, sandbox, workDir }) {
        const prepared = await sandbox.readTextFile({ path: join(workDir, 'input/source/package.json') })
        expect(prepared).toContain('example-package')
        await expect(sandbox.readTextFile({ path: join(workDir, 'consumer-config.json') })).resolves.toBe('{}\n')
        expect(options.instructions).toBeUndefined()
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/example-package/SKILL.md'),
          content: skillSource('example-package'),
        })
      },
    })
    const skillHarness = createSkillHarness({
      harness,
      sandbox: createFakeSandboxProvider(),
      sandboxConfig: {
        async onSession({ session, sessionWorkDir }) {
          configured = true
          await session.writeTextFile({ path: join(sessionWorkDir, 'consumer-config.json'), content: '{}\n' })
        },
      },
    })

    const result = await skillHarness.run({
      _tag: 'PackageSkill',
      source: { _tag: 'LocalPackage', rootDir: packageDir, packageDir: '.' },
      destination: { rootDir: destinationRoot, name: 'example-package' },
    })

    expect(result).toMatchObject({ _tag: 'Ok', value: { _tag: 'GeneratedSkill', name: 'example-package' } })
    await expect(readFile(join(destinationRoot, 'example-package/SKILL.md'), 'utf8')).resolves.toContain('name: example-package')
    expect(capture.starts).toHaveLength(1)
    expect(capture.starts[0]?.skills?.map(skill => skill.name)).toEqual(['generate-package-skill'])
    expect(capture.prompts).toHaveLength(1)
    expect(promptText(capture.prompts[0]?.prompt)).toContain('/input/source')
    expect(capture.destroyed).toBe(1)
    expect(configured).toBe(true)
  })

  it('rejects linked project files before starting an Agent', async () => {
    const projectDir = await mkdtemp(join(tmpdir(), 'skilld-project-'))
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const outside = join(projectDir, '..', 'skilld-outside-secret')
    await writeFile(join(projectDir, 'package.json'), '{"name":"example-project"}\n')
    await writeFile(outside, 'do not prepare')
    await symlink(outside, join(projectDir, 'linked-secret'))
    const { harness, capture } = createFakeHarness({ onPrompt: async () => {} })

    const result = await createSkillHarness({ harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'ProjectSkill',
      projectDir,
      destination: { rootDir: destinationRoot, name: 'example-project' },
    })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'SourceUnavailable' } })
    expect(capture.starts).toHaveLength(0)
  })

  it('returns a checked review result without promoting files', async () => {
    const skillDir = await mkdtemp(join(tmpdir(), 'skilld-review-'))
    await writeFile(join(skillDir, 'SKILL.md'), skillSource('review-me'))
    const { harness } = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/review/review.json'),
          content: JSON.stringify({
            summary: 'One warning.',
            findings: [{ level: 'warning', path: 'SKILL.md', message: 'Trigger is broad.', fix: 'Name one trigger.' }],
          }),
        })
      },
    })

    const result = await createSkillHarness({ harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'ReviewSkill',
      skillDir,
    })

    expect(result).toEqual({
      _tag: 'Ok',
      value: {
        _tag: 'SkillReview',
        summary: 'One warning.',
        findings: [{ level: 'warning', path: 'SKILL.md', message: 'Trigger is broad.', fix: 'Name one trigger.' }],
      },
    })
  })

  it('keeps the current Skill when generated output fails checks', async () => {
    const projectDir = await makePackage()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const currentDir = join(destinationRoot, 'example-project')
    await mkdir(currentDir)
    await writeFile(join(currentDir, 'SKILL.md'), 'current content\n')
    const { harness } = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/example-project/SKILL.md'),
          content: skillSource('wrong-name'),
        })
      },
    })

    const result = await createSkillHarness({ harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'ProjectSkill',
      projectDir,
      destination: { rootDir: destinationRoot, name: 'example-project' },
    })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'InvalidSkill' } })
    await expect(readFile(join(currentDir, 'SKILL.md'), 'utf8')).resolves.toBe('current content\n')
  })

  it('rejects a linked destination without replacing it', async () => {
    const projectDir = await makePackage()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const outside = await mkdtemp(join(tmpdir(), 'skilld-outside-'))
    await writeFile(join(outside, 'SKILL.md'), 'outside content\n')
    await symlink(outside, join(destinationRoot, 'example-project'))
    const { harness } = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/example-project/SKILL.md'),
          content: skillSource('example-project'),
        })
      },
    })

    const result = await createSkillHarness({ harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'ProjectSkill',
      projectDir,
      destination: { rootDir: destinationRoot, name: 'example-project' },
    })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'UnsafeOutputPath' } })
    await expect(readFile(join(outside, 'SKILL.md'), 'utf8')).resolves.toBe('outside content\n')
  })

  it('rejects linked Harness output and tags Agent failures', async () => {
    const projectDir = await makePackage()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const outside = join(projectDir, 'outside.md')
    await writeFile(outside, skillSource('example-project'))
    const linked = createFakeHarness({
      async onPrompt({ workDir }) {
        const outputDir = join(workDir, 'skilld-output/example-project')
        await mkdir(outputDir, { recursive: true })
        await symlink(outside, join(outputDir, 'SKILL.md'))
      },
    })

    const linkedResult = await createSkillHarness({ harness: linked.harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'ProjectSkill',
      projectDir,
      destination: { rootDir: destinationRoot, name: 'example-project' },
    })
    expect(linkedResult).toMatchObject({ _tag: 'Err', error: { _tag: 'InvalidSkill' } })

    const failed = createFakeHarness({ onPrompt: async () => {}, failPrompt: new Error('agent stopped') })
    const failedResult = await createSkillHarness({ harness: failed.harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'ProjectSkill',
      projectDir,
      destination: { rootDir: destinationRoot, name: 'example-project' },
    })
    expect(failedResult).toMatchObject({ _tag: 'Err', error: { _tag: 'AgentFailed' } })
  })

  it('returns OutputBusy when another Skill run holds the destination lock', async () => {
    const projectDir = await makePackage()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    await writeFile(join(destinationRoot, '.skilld-example-project.lock'), 'held\n')
    const { harness } = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/example-project/SKILL.md'),
          content: skillSource('example-project'),
        })
      },
    })

    const result = await createSkillHarness({ harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'ProjectSkill',
      projectDir,
      destination: { rootDir: destinationRoot, name: 'example-project' },
    })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'OutputBusy' } })
  })

  it('prepares the current Skill and replaces its directory atomically', async () => {
    const projectDir = await makePackage()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const currentDir = join(destinationRoot, 'example-project')
    await mkdir(join(currentDir, 'references'), { recursive: true })
    await writeFile(join(currentDir, 'SKILL.md'), skillSource('example-project', '# Old instructions\n'))
    await writeFile(join(currentDir, 'references/old.md'), 'old reference\n')
    const fake = createFakeHarness({
      async onPrompt({ sandbox, workDir, options }) {
        await expect(sandbox.readTextFile({ path: join(workDir, 'input/current-skill/SKILL.md') })).resolves.toContain('Old instructions')
        expect(promptText(options.prompt)).toContain('/input/current-skill')
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/example-project/SKILL.md'),
          content: skillSource('example-project', '# New instructions\n'),
        })
      },
    })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'ProjectSkill',
      projectDir,
      destination: { rootDir: destinationRoot, name: 'example-project' },
    })

    expect(result._tag).toBe('Ok')
    await expect(readFile(join(currentDir, 'SKILL.md'), 'utf8')).resolves.toContain('New instructions')
    await expect(readFile(join(currentDir, 'references/old.md'), 'utf8')).rejects.toMatchObject({ code: 'ENOENT' })
  })

  it('creates the project .skilld root for the self Skill workflow', async () => {
    const projectDir = await makePackage()
    const fake = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/self/SKILL.md'),
          content: skillSource('self'),
        })
      },
    })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'ProjectSkill',
      projectDir,
      destination: { rootDir: join(projectDir, '.skilld'), name: 'self' },
    })

    expect(result).toMatchObject({ _tag: 'Ok', value: { outputDir: join(projectDir, '.skilld/self') } })
    await expect(readFile(join(projectDir, '.skilld/self/SKILL.md'), 'utf8')).resolves.toContain('name: self')
  })

  it('returns Cancelled without starting an Agent', async () => {
    const projectDir = await makePackage()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const controller = new AbortController()
    controller.abort()
    const fake = createFakeHarness({ onPrompt: async () => {} })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'ProjectSkill',
      projectDir,
      destination: { rootDir: destinationRoot, name: 'example-project' },
    }, { signal: controller.signal })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'Cancelled' } })
    expect(fake.capture.starts).toHaveLength(0)
  })

  it('enforces a custom output file limit', async () => {
    const projectDir = await makePackage()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const fake = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        const outputDir = join(workDir, 'skilld-output/example-project')
        await sandbox.writeTextFile({ path: join(outputDir, 'SKILL.md'), content: skillSource('example-project') })
        await sandbox.writeTextFile({ path: join(outputDir, 'references/api.md'), content: '# API\n' })
      },
    })

    const result = await createSkillHarness({
      harness: fake.harness,
      sandbox: createFakeSandboxProvider(),
      outputPolicy: { maxOutputFiles: 1 },
    }).run({
      _tag: 'ProjectSkill',
      projectDir,
      destination: { rootDir: destinationRoot, name: 'example-project' },
    })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'InvalidSkill' } })
  })
})
