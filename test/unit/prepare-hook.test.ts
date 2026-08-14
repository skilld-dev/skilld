import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { buildPrepareScript } from '../../src/cli-helpers.ts'
import { editJsonProperty } from '../../src/core/package-json.ts'

function makeTempCwd(hasSkilld: boolean): string {
  const dir = mkdtempSync(join(tmpdir(), 'prepare-hook-'))
  const pkg: Record<string, any> = { name: 'test-pkg' }
  if (hasSkilld)
    pkg.devDependencies = { skilld: '^1.0.0' }
  writeFileSync(join(dir, 'package.json'), JSON.stringify(pkg))
  return dir
}

describe('prepare hook script building', () => {
  const cwdWithSkilld = makeTempCwd(true)
  const cwdWithout = makeTempCwd(false)
  const standalone = 'skilld prepare || true'
  const npxStandalone = 'npx skilld prepare || true'

  it('uses skilld when installed as dependency', () => {
    expect(buildPrepareScript(undefined, cwdWithSkilld)).toBe(standalone)
  })

  it('uses npx skilld when not installed as dependency', () => {
    expect(buildPrepareScript(undefined, cwdWithout)).toBe(npxStandalone)
  })

  it('returns standalone when no existing script', () => {
    expect(buildPrepareScript(undefined, cwdWithSkilld)).toBe(standalone)
  })

  it('returns standalone when existing script is empty', () => {
    expect(buildPrepareScript('', cwdWithSkilld)).toBe(standalone)
    expect(buildPrepareScript('   ', cwdWithSkilld)).toBe(standalone)
  })

  it('appends with && and parens to existing script', () => {
    expect(buildPrepareScript('husky', cwdWithSkilld)).toBe('husky && (skilld prepare || true)')
  })

  it('handles existing script with multiple commands', () => {
    expect(buildPrepareScript('husky && lint-staged', cwdWithSkilld)).toBe('husky && lint-staged && (skilld prepare || true)')
  })

  it('strips trailing && from existing script', () => {
    expect(buildPrepareScript('husky &&', cwdWithSkilld)).toBe('husky && (skilld prepare || true)')
    expect(buildPrepareScript('husky && ', cwdWithSkilld)).toBe('husky && (skilld prepare || true)')
  })

  it('strips trailing ; from existing script', () => {
    expect(buildPrepareScript('husky;', cwdWithSkilld)).toBe('husky && (skilld prepare || true)')
  })

  it('strips trailing || from existing script', () => {
    expect(buildPrepareScript('husky ||', cwdWithSkilld)).toBe('husky && (skilld prepare || true)')
  })

  it('handles only operators as existing script', () => {
    expect(buildPrepareScript('&&', cwdWithSkilld)).toBe(standalone)
    expect(buildPrepareScript(';', cwdWithSkilld)).toBe(standalone)
  })

  it('appends npx variant to existing script when not installed', () => {
    expect(buildPrepareScript('husky', cwdWithout)).toBe('husky && (npx skilld prepare || true)')
  })

  describe('surgical package.json editing', () => {
    it('adds prepare to empty scripts object', () => {
      const raw = `{
  "name": "my-pkg",
  "scripts": {
    "build": "tsc"
  }
}
`
      const result = editJsonProperty(raw, ['scripts', 'prepare'], standalone)
      expect(result).toContain(`"prepare": "${standalone}"`)
      expect(result).toContain('"build": "tsc"')
    })

    it('adds scripts object when missing', () => {
      const raw = `{
  "name": "my-pkg"
}
`
      let result = editJsonProperty(raw, ['scripts'], {})
      result = editJsonProperty(result, ['scripts', 'prepare'], standalone)
      expect(result).toContain('"scripts"')
      expect(result).toContain(`"prepare": "${standalone}"`)
      expect(result).toContain('"name": "my-pkg"')
    })

    it('replaces existing prepare script preserving formatting', () => {
      const raw = `{
  "name": "my-pkg",
  "scripts": {
    "prepare": "husky",
    "build": "tsc"
  }
}
`
      const result = editJsonProperty(raw, ['scripts', 'prepare'], 'husky && (skilld prepare || true)')
      expect(result).toContain('"prepare": "husky && (skilld prepare || true)"')
      expect(result).toContain('"build": "tsc"')
      expect(result).toContain('"name": "my-pkg"')
    })
  })
})
