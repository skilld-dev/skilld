import { mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'pathe'
import { afterEach, describe, expect, it } from 'vitest'
import { getShippedSkills, resolvePkgDir } from '../../src/core/prepare.ts'

describe('package dir probing', () => {
  const fixtureDirs: string[] = []

  afterEach(() => {
    for (const dir of fixtureDirs)
      rmSync(dir, { recursive: true, force: true })
    fixtureDirs.length = 0
  })

  it.each(['npm:vue', 'gh:owner/repo', ''])('returns null for %j', (name) => {
    expect(resolvePkgDir(name, process.cwd(), '1.0.0')).toBeNull()
  })

  it('rejects traversal when the escaped directory exists', () => {
    const cwd = mkdtempSync(join(tmpdir(), 'skilld-pkg-probe-'))
    fixtureDirs.push(cwd)
    mkdirSync(join(cwd, 'escape'))

    expect(resolvePkgDir('../escape', cwd, '1.0.0')).toBeNull()
  })

  it('returns an installed package before validating the cache version', () => {
    const cwd = mkdtempSync(join(tmpdir(), 'skilld-pkg-probe-'))
    fixtureDirs.push(cwd)
    const packageDir = join(cwd, 'node_modules', 'vue')
    mkdirSync(packageDir, { recursive: true })

    expect(resolvePkgDir('vue', cwd, '../invalid')).toBe(packageDir)
  })

  it.each(['npm:vue', '../escape'])('reports no shipped skills for %j', (name) => {
    expect(getShippedSkills(name, process.cwd(), '1.0.0')).toEqual([])
  })
})
