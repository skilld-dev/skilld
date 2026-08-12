import { describe, expect, it } from 'vitest'
import { getShippedSkills, resolvePkgDir } from '../../src/core/prepare.ts'

describe('package dir probing', () => {
  it.each(['npm:vue', 'gh:owner/repo', '../escape', ''])('returns null for %j', (name) => {
    expect(resolvePkgDir(name, process.cwd(), '1.0.0')).toBeNull()
  })

  it.each(['npm:vue', '../escape'])('reports no shipped skills for %j', (name) => {
    expect(getShippedSkills(name, process.cwd(), '1.0.0')).toEqual([])
  })
})
