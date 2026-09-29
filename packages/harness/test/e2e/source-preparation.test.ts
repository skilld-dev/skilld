import { mkdir, mkdtemp, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createSkillHarness } from '../../src/index.ts'
import { createFakeHarness, createFakeSandboxProvider, skillSource } from '../support/fakes.ts'

async function makePackage(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), 'skilld-source-'))
  await writeFile(join(root, 'package.json'), '{"name":"example-package"}\n')
  await writeFile(join(root, 'README.md'), '# Example package\n')
  return root
}

describe('local source preparation', () => {
  it('names the linked path it rejects', async () => {
    const packageDir = await makePackage()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    await mkdir(join(packageDir, 'src'))
    await symlink(join(packageDir, 'README.md'), join(packageDir, 'src/devtools'))
    const fake = createFakeHarness({ onPrompt: async () => {} })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'LocalPackage', rootDir: packageDir, packageDir: '.' },
      destination: { rootDir: destinationRoot, name: 'example-package' },
    })

    if (result._tag !== 'Err' || result.error._tag !== 'SourceUnavailable')
      throw new Error('Expected a SourceUnavailable error.')
    expect(result.error.message).toContain('src/devtools')
    expect(result.error.attempts[0]?.reason).toContain('src/devtools')
  })

  it('reports each source file it leaves out for size', async () => {
    const packageDir = await makePackage()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    await writeFile(join(packageDir, 'pnpm-lock.yaml'), 'x'.repeat(2048))
    let manifest = ''
    const fake = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        manifest = await sandbox.readTextFile({ path: join(workDir, 'input/source-manifest.json') }) ?? ''
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/example-package/SKILL.md'),
          content: skillSource('example-package'),
        })
      },
    })

    const result = await createSkillHarness({
      harness: fake.harness,
      sandbox: createFakeSandboxProvider(),
      outputPolicy: { maxSourceFileBytes: 1024 },
    }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'LocalPackage', rootDir: packageDir, packageDir: '.' },
      destination: { rootDir: destinationRoot, name: 'example-package' },
    })

    if (result._tag !== 'Ok' || result.value._tag !== 'GeneratedSkill')
      throw new Error('Expected a GeneratedSkill.')
    expect(result.report.warnings).toEqual([
      'Source file pnpm-lock.yaml was left out: 2048 bytes exceeds the 1024 byte file limit.',
    ])
    expect(JSON.parse(manifest).skippedFiles).toEqual([{ path: 'pnpm-lock.yaml', bytes: 2048 }])
  })

  it('reports a file it leaves out of a reviewed Skill', async () => {
    const skillDir = await mkdtemp(join(tmpdir(), 'skilld-review-'))
    await writeFile(join(skillDir, 'SKILL.md'), skillSource('review-me'))
    await writeFile(join(skillDir, 'large.txt'), 'x'.repeat(2048))
    const fake = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/review/review.json'),
          content: JSON.stringify({ summary: 'Clean.', findings: [] }),
        })
      },
    })

    const result = await createSkillHarness({
      harness: fake.harness,
      sandbox: createFakeSandboxProvider(),
      outputPolicy: { maxSourceFileBytes: 1024 },
    }).run({ _tag: 'ReviewSkill', skillDir })

    expect(result).toEqual({
      _tag: 'Ok',
      value: {
        _tag: 'SkillReview',
        summary: 'Clean.',
        findings: [],
      },
      report: {
        usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 },
        steps: 1,
        warnings: ['Source file large.txt was left out: 2048 bytes exceeds the 1024 byte file limit.'],
      },
    })
  })
})

describe('sandbox session lifecycle', () => {
  it('destroys the sandbox session it creates and prints no deprecation warning', async () => {
    const packageDir = await makePackage()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const provider = createFakeSandboxProvider()
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const fake = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/example-package/SKILL.md'),
          content: skillSource('example-package'),
        })
      },
    })

    try {
      const result = await createSkillHarness({ harness: fake.harness, sandbox: provider }).run({
        _tag: 'PackageSkill',
        source: { _tag: 'LocalPackage', rootDir: packageDir, packageDir: '.' },
        destination: { rootDir: destinationRoot, name: 'example-package' },
      })
      expect(result._tag).toBe('Ok')
      expect(provider.destroyed()).toBe(1)
      expect(warn.mock.calls.flat().join('\n')).not.toContain('deprecated')
    }
    finally {
      warn.mockRestore()
    }
  })

  it('destroys the sandbox session when the Agent fails to start', async () => {
    const packageDir = await makePackage()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const provider = createFakeSandboxProvider()
    const fake = createFakeHarness({ onPrompt: async () => {}, failStart: new Error('no agent') })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: provider }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'LocalPackage', rootDir: packageDir, packageDir: '.' },
      destination: { rootDir: destinationRoot, name: 'example-package' },
    })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'AgentFailed' } })
    expect(provider.destroyed()).toBe(1)
  })
  it('reports a sandbox cleanup failure on a failed run', async () => {
    const provider = createFakeSandboxProvider()
    const failingProvider = {
      ...provider,
      createSession: async () => ({
        ...await provider.createSession(),
        destroy: async () => {
          throw new Error('sandbox gone')
        },
      }),
    }
    const fake = createFakeHarness({ onPrompt: async () => {}, failStart: new Error('no agent') })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: failingProvider }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'LocalPackage', rootDir: await makePackage(), packageDir: '.' },
      destination: { rootDir: await mkdtemp(join(tmpdir(), 'skilld-output-')), name: 'example-package' },
    })

    expect(result).toMatchObject({
      _tag: 'Err',
      error: { _tag: 'AgentFailed' },
      report: { warnings: ['Sandbox session cleanup failed: sandbox gone'] },
    })
  })
})
