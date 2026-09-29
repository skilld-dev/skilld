import { mkdir, mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { harnessSkillNames, loadSkilldMaintainedSkill, skilldMaintainedSkillNames } from '../../src/skills.ts'

describe('skilld-maintained Skills', () => {
  it('loads the runnable Harness Skills', async () => {
    await expect(harnessSkillNames()).resolves.toEqual([
      'generate-package-skill',
      'generate-project-skill',
      'review-skill',
    ])
  })

  it('loads visible Harness request assets', async () => {
    const skill = await loadSkilldMaintainedSkill('generate-package-skill')

    expect(skill.name).toBe('generate-package-skill')
    expect(skill.files).toEqual(expect.arrayContaining([
      expect.objectContaining({ path: 'assets/harness-request.md' }),
    ]))
  })

  it('loads the scripts a Harness Skill links', async () => {
    const skill = await loadSkilldMaintainedSkill('generate-package-skill')
    const script = skill.files?.find(file => file.path === 'scripts/serve-fixture.mjs')

    expect(script?.content).toContain('Usage: node serve-fixture.mjs')
  })

  it('loads nested scripts and references for the Agent', async () => {
    const root = await mkdtemp(join(tmpdir(), 'skilld-skills-'))
    const files: Record<string, string> = {
      'skilld-maintained-skills.json': '["nested-skill"]',
      'harness-skills.json': '["nested-skill"]',
      'nested-skill/SKILL.md': '---\nname: nested-skill\ndescription: Test Skill.\n---\n\nBody.\n',
      'nested-skill/assets/harness-request.md': 'Request.\n',
      'nested-skill/scripts/run.mjs': 'run\n',
      'nested-skill/scripts/lib/shared.mjs': 'shared\n',
      'nested-skill/references/api.md': 'api\n',
    }
    for (const [path, content] of Object.entries(files)) {
      await mkdir(join(root, path, '..'), { recursive: true })
      await writeFile(join(root, path), content)
    }

    const skill = await loadSkilldMaintainedSkill('nested-skill', [root])

    expect(skill.files?.map(file => file.path).sort()).toEqual([
      'assets/harness-request.md',
      'references/api.md',
      'scripts/lib/shared.mjs',
      'scripts/run.mjs',
    ])
    expect(skill.files?.find(file => file.path === 'scripts/lib/shared.mjs')?.content).toBe('shared\n')
  })

  it('loads the direct skilld Skill without a Harness request', async () => {
    const skill = await loadSkilldMaintainedSkill('skilld')

    expect(skill.name).toBe('skilld')
    expect(skill.files).toBeUndefined()
  })

  it('lists every published skilld-maintained Skill', async () => {
    await expect(skilldMaintainedSkillNames()).resolves.toEqual([
      'generate-package-skill',
      'generate-project-skill',
      'review-skill',
      'skilld',
    ])
  })
})
