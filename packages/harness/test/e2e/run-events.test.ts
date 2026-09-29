import type { GeneratedSkill, SkillReview, SkillRunEvent } from '../../src/index.ts'
import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createSkillHarness } from '../../src/index.ts'
import { createFakeHarness, createFakeSandboxProvider, skillSource } from '../support/fakes.ts'

const usage = {
  inputTokens: { total: 120, noCache: 20, cacheRead: 100, cacheWrite: 0 },
  outputTokens: { total: 30, text: 30, reasoning: 0 },
}

async function makePackage() {
  const root = await mkdtemp(join(tmpdir(), 'skilld-package-'))
  await writeFile(join(root, 'package.json'), '{"name":"example-package"}\n')
  return root
}

function packageHarness() {
  const { harness } = createFakeHarness({
    async onPrompt({ sandbox, workDir }) {
      await sandbox.writeTextFile({
        path: join(workDir, 'skilld-output/example-package/SKILL.md'),
        content: skillSource('example-package'),
      })
    },
    parts: [
      { type: 'tool-call', toolCallId: 'call-1', toolName: 'read', input: '{"path":"package.json"}', providerExecuted: true },
      { type: 'tool-result', toolCallId: 'call-1', toolName: 'read', result: '{}' },
    ],
    usage,
  })
  return { skillHarness: createSkillHarness({ harness, sandbox: createFakeSandboxProvider() }) }
}

describe('skill run progress', () => {
  it('forwards step and tool call events as tagged events', async () => {
    const events: SkillRunEvent[] = []
    const { skillHarness } = packageHarness()

    await skillHarness.run({
      _tag: 'PackageSkill',
      source: { _tag: 'LocalPackage', rootDir: await makePackage(), packageDir: '.' },
      destination: { rootDir: await mkdtemp(join(tmpdir(), 'skilld-output-')), name: 'example-package' },
    }, { onEvent: event => events.push(event) })

    expect(events.map(event => event._tag)).toEqual(['StepStart', 'ToolCall', 'StepFinish'])
    expect(events[1]).toMatchObject({ _tag: 'ToolCall', step: 0, toolName: 'read', toolCallId: 'call-1' })
    expect(events[2]).toMatchObject({ _tag: 'StepFinish', step: 0 })
  })

  it('returns usage and the step count on a generated Skill', async () => {
    const { skillHarness } = packageHarness()

    const result = await skillHarness.run({
      _tag: 'PackageSkill',
      source: { _tag: 'LocalPackage', rootDir: await makePackage(), packageDir: '.' },
      destination: { rootDir: await mkdtemp(join(tmpdir(), 'skilld-output-')), name: 'example-package' },
    })

    expect(result).toMatchObject({
      _tag: 'Ok',
      value: { steps: 1, usage: { inputTokens: 120, cachedInputTokens: 100, outputTokens: 30 } },
    })
  })

  it('reports an onEvent failure as a warning and still finishes the run', async () => {
    const { skillHarness } = packageHarness()

    const result = await skillHarness.run({
      _tag: 'PackageSkill',
      source: { _tag: 'LocalPackage', rootDir: await makePackage(), packageDir: '.' },
      destination: { rootDir: await mkdtemp(join(tmpdir(), 'skilld-output-')), name: 'example-package' },
    }, {
      onEvent: () => {
        throw new Error('progress bar broke')
      },
    })

    expect(result._tag).toBe('Ok')
    if (result._tag === 'Ok')
      expect(result.value.warnings).toEqual(['onEvent failed: progress bar broke'])
  })

  it('reports a rejected async onEvent as a warning', async () => {
    const { skillHarness } = packageHarness()

    const result = await skillHarness.run({
      _tag: 'PackageSkill',
      source: { _tag: 'LocalPackage', rootDir: await makePackage(), packageDir: '.' },
      destination: { rootDir: await mkdtemp(join(tmpdir(), 'skilld-output-')), name: 'example-package' },
    }, {
      onEvent: async () => {
        throw new Error('remote log down')
      },
    })

    expect(result._tag).toBe('Ok')
    if (result._tag === 'Ok')
      expect(result.value.warnings).toEqual(['onEvent failed: remote log down'])
  })

  it('returns usage on a Skill review', async () => {
    const skillDir = await mkdtemp(join(tmpdir(), 'skilld-review-'))
    await writeFile(join(skillDir, 'SKILL.md'), skillSource('review-me'))
    const { harness } = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/review/review.json'),
          content: JSON.stringify({ summary: 'Clean.', findings: [] }),
        })
      },
      usage,
    })

    const result = await createSkillHarness({ harness, sandbox: createFakeSandboxProvider() }).run({ _tag: 'ReviewSkill', skillDir })

    expect(result).toMatchObject({ _tag: 'Ok', value: { _tag: 'SkillReview', steps: 1, usage: { outputTokens: 30 } } })
  })
})

describe('skill run result type', () => {
  it('narrows the value by the input tag', async () => {
    const { skillHarness } = packageHarness()
    const run = () => skillHarness.run({
      _tag: 'PackageSkill',
      source: { _tag: 'NpmPackage', spec: 'example-package@1.0.0' },
      destination: { rootDir: '/tmp', name: 'example-package' },
    })
    const review = () => skillHarness.run({ _tag: 'ReviewSkill', skillDir: '/tmp' })

    expectTypeOf(run).returns.resolves.extract<{ _tag: 'Ok' }>().toHaveProperty('value').toEqualTypeOf<GeneratedSkill>()
    expectTypeOf(review).returns.resolves.extract<{ _tag: 'Ok' }>().toHaveProperty('value').toEqualTypeOf<SkillReview>()
  })
})
