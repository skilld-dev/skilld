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
