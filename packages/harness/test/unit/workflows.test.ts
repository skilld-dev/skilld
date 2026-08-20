import { harnessWorkflowNames, loadSkilldMaintainedSkill } from '../../src/workflows.ts'

describe('skilld-maintained Skills', () => {
  it('loads the runnable Harness workflows', async () => {
    await expect(harnessWorkflowNames()).resolves.toEqual([
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
})
