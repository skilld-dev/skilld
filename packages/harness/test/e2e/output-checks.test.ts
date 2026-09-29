import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createSkillHarness } from '../../src/index.ts'
import { createFakeHarness, createFakeSandboxProvider, projectSkillBody } from '../support/fakes.ts'

type Output = Readonly<Record<string, string>>

function frontmatter(name: string, extra = ''): string {
  return `---\nname: ${name}\ndescription: Use ${name} when working with its public API.\n${extra}---\n\n`
}

async function runPackageSkill(output: Output) {
  const packageDir = await mkdtemp(join(tmpdir(), 'skilld-checks-'))
  await writeFile(join(packageDir, 'package.json'), '{"name":"example-package"}\n')
  const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-checks-out-'))
  const fake = createFakeHarness({
    async onPrompt({ sandbox, workDir }) {
      for (const [path, content] of Object.entries(output))
        await sandbox.writeTextFile({ path: join(workDir, 'skilld-output/example-package', path), content })
    },
  })
  return createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider() }).run({
    _tag: 'PackageSkill',
    source: { _tag: 'LocalPackage', rootDir: packageDir, packageDir: '.' },
    destination: { rootDir: destinationRoot, name: 'example-package' },
  })
}

async function runProjectSkill(output: Output) {
  const projectDir = await mkdtemp(join(tmpdir(), 'skilld-checks-'))
  await writeFile(join(projectDir, 'package.json'), '{"name":"example-project"}\n')
  const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-checks-out-'))
  const fake = createFakeHarness({
    async onPrompt({ sandbox, workDir }) {
      for (const [path, content] of Object.entries(output))
        await sandbox.writeTextFile({ path: join(workDir, 'skilld-output/example-project', path), content })
    },
  })
  return createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider() }).run({
    _tag: 'ProjectSkill',
    projectDir,
    destination: { rootDir: destinationRoot, name: 'example-project' },
  })
}

function issues(result: Awaited<ReturnType<typeof runPackageSkill>>): ReadonlyArray<string> {
  if (result._tag !== 'Err' || result.error._tag !== 'InvalidSkill')
    throw new Error(`Expected an InvalidSkill error, got ${JSON.stringify(result)}`)
  return result.error.issues
}

function references(count: number): Output {
  return Object.fromEntries(Array.from({ length: count }, (_, index) => [`references/topic-${index + 1}.md`, `# Topic ${index + 1}\n`]))
}

function linking(paths: ReadonlyArray<string>): string {
  return paths.map(path => `- [${path}](${path})`).join('\n')
}

describe('generated Skill output checks', () => {
  it('promotes a package Skill that links eight references', async () => {
    const files = references(8)
    const result = await runPackageSkill({
      'SKILL.md': `${frontmatter('example-package')}# Example\n\n${linking(Object.keys(files))}\n`,
      ...files,
    })

    expect(result).toMatchObject({ _tag: 'Ok', value: { _tag: 'GeneratedSkill' } })
  })

  it.each([
    ['license: MIT\n', 'license'],
    ['metadata:\n  owner: example\n', 'metadata'],
  ])('rejects a frontmatter field other than name and description (%s)', async (extra, field) => {
    const result = await runPackageSkill({ 'SKILL.md': `${frontmatter('example-package', extra)}# Example\n` })

    expect(issues(result)).toContain(`Frontmatter field is not supported: ${field}`)
  })

  it('rejects a package SKILL.md of 500 lines or more', async () => {
    const body = Array.from({ length: 500 }, (_, index) => `Line ${index + 1}.`).join('\n')
    const result = await runPackageSkill({ 'SKILL.md': `${frontmatter('example-package')}${body}\n` })

    expect(issues(result)).toContain('SKILL.md must stay under 500 lines; it has 505.')
  })

  it('rejects a package Skill with more than eight references', async () => {
    const files = references(9)
    const result = await runPackageSkill({
      'SKILL.md': `${frontmatter('example-package')}# Example\n\n${linking(Object.keys(files))}\n`,
      ...files,
    })

    expect(issues(result)).toContain('A package Skill may write at most 8 reference files; it wrote 9.')
  })

  it('rejects a reference that SKILL.md does not link', async () => {
    const result = await runPackageSkill({
      'SKILL.md': `${frontmatter('example-package')}# Example\n\n- [API](references/api.md)\n`,
      'references/api.md': '# API\n',
      'references/orphan.md': '# Orphan\n',
    })

    expect(issues(result)).toEqual(['SKILL.md must link references/orphan.md.'])
  })

  it('accepts a reference named in inline code or linked with an anchor', async () => {
    const result = await runPackageSkill({
      'SKILL.md': `${frontmatter('example-package')}# Example\n\nRead \`references/api.md\`.\nSee [errors](./references/errors.md#codes).\n`,
      'references/api.md': '# API\n',
      'references/errors.md': '# Errors\n',
    })

    expect(result).toMatchObject({ _tag: 'Ok' })
  })

  it('applies the frontmatter and link rules to a project Skill', async () => {
    const result = await runProjectSkill({
      'SKILL.md': `${frontmatter('example-project', 'license: MIT\n')}${projectSkillBody()}`,
      'references/orphan.md': '# Orphan\n',
    })

    expect(issues(result)).toEqual([
      'Frontmatter field is not supported: license',
      'SKILL.md must link references/orphan.md.',
    ])
  })
})
