import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createSkillHarness } from '../../src/index.ts'
import { createFakeHarness, createFakeSandboxProvider } from '../support/fakes.ts'

async function makeProject(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), 'skilld-project-nav-'))
  await writeFile(join(root, 'package.json'), '{"name":"example-project","main":"./src/index.ts"}\n')
  await writeFile(join(root, 'README.md'), '# Example project\n')
  await mkdir(join(root, 'src'))
  await writeFile(join(root, 'src/index.ts'), 'export const start = () => true\n')
  await mkdir(join(root, 'dist'))
  await writeFile(join(root, 'dist/index.js'), 'export const start=()=>true\n')
  return root
}

function projectSkill(body: string): string {
  return `---\nname: example-project\ndescription: Work on the example project through its real files.\n---\n\n${body}`
}

const conforming = [
  '# Example project',
  '',
  'The entry point is `src/index.ts`. The manifest is `package.json`.',
  '',
  'Search the source:',
  '',
  '```sh',
  'rg -n "export " src',
  '```',
  '',
].join('\n')

async function runProjectSkill(markdown: string): Promise<{
  result: Awaited<ReturnType<ReturnType<typeof createSkillHarness>['run']>>
  destinationRoot: string
}> {
  const projectDir = await makeProject()
  const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-project-nav-out-'))
  const { harness } = createFakeHarness({
    async onPrompt({ sandbox, workDir }) {
      await sandbox.writeTextFile({
        path: join(workDir, 'skilld-output/example-project/SKILL.md'),
        content: markdown,
      })
    },
  })
  const result = await createSkillHarness({ harness, sandbox: createFakeSandboxProvider() }).run({
    _tag: 'ProjectSkill',
    projectDir,
    destination: { rootDir: destinationRoot, name: 'example-project' },
  })
  return { result, destinationRoot }
}

describe('project Skill navigation checks', () => {
  it('promotes a project Skill whose pointers and searches hold', async () => {
    const { result, destinationRoot } = await runProjectSkill(projectSkill(conforming))

    expect(result).toMatchObject({ _tag: 'Ok', value: { _tag: 'GeneratedSkill' } })
    await expect(readFile(join(destinationRoot, 'example-project/SKILL.md'), 'utf8')).resolves.toContain('src/index.ts')
  })

  it('refuses a pointer the project does not contain', async () => {
    const body = conforming.replace('`src/index.ts`', '`src/server/boot.ts`')

    const { result } = await runProjectSkill(projectSkill(body))

    expect(result._tag).toBe('Err')
    if (result._tag !== 'Err' || result.error._tag !== 'InvalidSkill')
      throw new Error('Expected an InvalidSkill error.')
    expect(result.error.issues).toContain('SKILL.md points at a path the project does not contain: src/server/boot.ts.')
  })

  it('accepts a path a line reports as missing', async () => {
    const body = conforming.replace(
      'The entry point is `src/index.ts`.',
      'The entry point is `src/index.ts`. The project has no `src/lib.ts`, so the test cannot import one.',
    )

    const { result } = await runProjectSkill(projectSkill(body))

    expect(result).toMatchObject({ _tag: 'Ok', value: { _tag: 'GeneratedSkill' } })
  })

  it('accepts a denial sentence wrapped across two prose lines', async () => {
    const body = conforming.replace(
      'The entry point is `src/index.ts`. The manifest is `package.json`.',
      'The project lacks\n`src/store.ts` so the import must be added first.',
    )

    const { result } = await runProjectSkill(projectSkill(body))

    expect(result).toMatchObject({ _tag: 'Ok', value: { _tag: 'GeneratedSkill' } })
  })

  it('accepts a generated path a line puts off limits', async () => {
    const body = conforming.replace(
      'The entry point is `src/index.ts`.',
      'The entry point is `src/index.ts`. Never edit `dist/index.js`, which the build writes.',
    )

    const { result } = await runProjectSkill(projectSkill(body))

    expect(result).toMatchObject({ _tag: 'Ok', value: { _tag: 'GeneratedSkill' } })
  })

  it('refuses an invented path a line presents as real', async () => {
    const body = conforming.replace(
      'The entry point is `src/index.ts`.',
      'The entry point is `src/index.ts`. Request handling lives in `src/server/routes.ts`.',
    )

    const { result } = await runProjectSkill(projectSkill(body))

    if (result._tag !== 'Err' || result.error._tag !== 'InvalidSkill')
      throw new Error('Expected an InvalidSkill error.')
    expect(result.error.issues).toContain('SKILL.md points at a path the project does not contain: src/server/routes.ts.')
  })

  it('accepts a version and a product name in inline code', async () => {
    const body = conforming.replace(
      'The entry point is `src/index.ts`. The manifest is `package.json`.',
      'The entry point is `src/index.ts`. The manifest is `package.json`. The project runs on `Node.js` `20.19.0`. The runtime is `node.js` and `vue.js`.',
    )

    const { result } = await runProjectSkill(projectSkill(body))

    expect(result).toMatchObject({ _tag: 'Ok', value: { _tag: 'GeneratedSkill' } })
  })

  it('accepts a file a line tells the Agent to copy from a template', async () => {
    const projectDir = await makeProject()
    await writeFile(join(projectDir, '.env.example'), 'SECRET=\n')
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-project-nav-out-'))
    const body = [
      '# Example project',
      '',
      'The entry point is `src/index.ts`.',
      '',
      'Copy `.env.example` to `.env` for local secrets.',
      '',
      '```sh',
      'rg -n "export " src',
      '```',
      '',
    ].join('\n')
    const { harness } = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/example-project/SKILL.md'),
          content: projectSkill(body),
        })
      },
    })

    const result = await createSkillHarness({ harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'ProjectSkill',
      projectDir,
      destination: { rootDir: destinationRoot, name: 'example-project' },
    })

    expect(result).toMatchObject({ _tag: 'Ok', value: { _tag: 'GeneratedSkill' } })
  })

  it('accepts a bare inline search command with no arguments', async () => {
    const body = [
      '# Example project',
      '',
      'The entry point is `src/index.ts`.',
      '',
      'Search the source with `rg`.',
      '',
    ].join('\n')

    const { result } = await runProjectSkill(projectSkill(body))

    expect(result).toMatchObject({ _tag: 'Ok', value: { _tag: 'GeneratedSkill' } })
  })

  it('refuses a project Skill that gives the Agent no search command', async () => {
    const body = ['# Example project', '', 'The entry point is `src/index.ts`.', ''].join('\n')

    const { result } = await runProjectSkill(projectSkill(body))

    if (result._tag !== 'Err' || result.error._tag !== 'InvalidSkill')
      throw new Error('Expected an InvalidSkill error.')
    expect(result.error.issues).toContain('SKILL.md must give the Agent at least one search command it can repeat.')
  })

  it('accepts a directory pointer, a glob, and a file it writes beside SKILL.md', async () => {
    const body = [
      '# Example project',
      '',
      'Source lives in `src/`. Types live in `src/**/*.ts`.',
      'Architecture notes are in `references/architecture.md`.',
      '',
      '```sh',
      'rg -n "export " src',
      '```',
      '',
    ].join('\n')
    const projectDir = await makeProject()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-project-nav-out-'))
    const { harness } = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/example-project/SKILL.md'),
          content: projectSkill(body),
        })
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/example-project/references/architecture.md'),
          content: '# Architecture\n',
        })
      },
    })

    const result = await createSkillHarness({ harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'ProjectSkill',
      projectDir,
      destination: { rootDir: destinationRoot, name: 'example-project' },
    })

    expect(result).toMatchObject({ _tag: 'Ok', value: { _tag: 'GeneratedSkill' } })
  })

  it('leaves a package Skill unchecked for project navigation', async () => {
    const projectDir = await makeProject()
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-package-nav-out-'))
    const body = ['# Example package', '', 'Read `src/server/boot.ts` for the entry point.', ''].join('\n')
    const { harness } = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/example-package/SKILL.md'),
          content: `---\nname: example-package\ndescription: Use the example package through its public API.\n---\n\n${body}`,
        })
      },
    })

    const result = await createSkillHarness({ harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'LocalPackage', rootDir: projectDir, packageDir: '.' },
      destination: { rootDir: destinationRoot, name: 'example-package' },
    })

    expect(result).toMatchObject({ _tag: 'Ok', value: { _tag: 'GeneratedSkill' } })
  })
})
