import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { invalidateLockCache } from '../../src/core/lockfile.ts'
import { readProjectLock } from '../../src/core/skills.ts'

let cwd: string

function writeLockfile(dir: string, skills: Record<string, { packageName: string, version: string, syncedAt?: string }>): void {
  mkdirSync(join(cwd, dir), { recursive: true })
  let yaml = 'skills:\n'
  for (const [name, info] of Object.entries(skills)) {
    yaml += `  ${name}:\n`
    yaml += `    packageName: ${info.packageName}\n`
    yaml += `    version: ${info.version}\n`
    if (info.syncedAt)
      yaml += `    syncedAt: ${info.syncedAt}\n`
  }
  writeFileSync(join(cwd, dir, 'skilld-lock.yaml'), yaml)
}

function packages(lock: ReturnType<typeof readProjectLock>): string[] {
  return Object.values(lock?.skills ?? {}).map(s => `${s.packageName}@${s.version}`).sort()
}

beforeEach(() => {
  cwd = mkdtempSync(join(tmpdir(), 'skilld-lock-'))
  invalidateLockCache()
})

afterEach(() => {
  rmSync(cwd, { recursive: true, force: true })
  invalidateLockCache()
})

describe('readProjectLock', () => {
  it('returns null when no agent has a lockfile', () => {
    expect(readProjectLock(cwd)).toBeNull()
  })

  it('reads a single agent dir', () => {
    writeLockfile('.claude/skills', { vue: { packageName: 'vue', version: '3.5.0' } })
    expect(packages(readProjectLock(cwd))).toEqual(['vue@3.5.0'])
  })

  it('merges every agent dir', () => {
    writeLockfile('.claude/skills', { vue: { packageName: 'vue', version: '3.5.0' } })
    writeLockfile('.agents/skills', { zod: { packageName: 'zod', version: '3.23.0' } })
    expect(packages(readProjectLock(cwd))).toEqual(['vue@3.5.0', 'zod@3.23.0'])
  })

  it('dedupes a skill present in several agent dirs, preferring the newest sync', () => {
    writeLockfile('.claude/skills', { vue: { packageName: 'vue', version: '3.4.0', syncedAt: '2026-01-01' } })
    writeLockfile('.cursor/skills', { vue: { packageName: 'vue', version: '3.5.0', syncedAt: '2026-06-01' } })
    expect(packages(readProjectLock(cwd))).toEqual(['vue@3.5.0'])
  })

  it('restricts to the requested agents', () => {
    writeLockfile('.claude/skills', { vue: { packageName: 'vue', version: '3.5.0' } })
    writeLockfile('.agents/skills', { zod: { packageName: 'zod', version: '3.23.0' } })

    expect(packages(readProjectLock(cwd, ['claude-code']))).toEqual(['vue@3.5.0'])
    expect(packages(readProjectLock(cwd, ['codex']))).toEqual(['zod@3.23.0'])
    expect(packages(readProjectLock(cwd, ['claude-code', 'codex']))).toEqual(['vue@3.5.0', 'zod@3.23.0'])
  })

  it('returns null when the requested agent has no lockfile', () => {
    writeLockfile('.claude/skills', { vue: { packageName: 'vue', version: '3.5.0' } })
    expect(readProjectLock(cwd, ['cursor'])).toBeNull()
  })

  it('prefers a shared skills dir over agent dirs', () => {
    writeLockfile('.skills', { vue: { packageName: 'vue', version: '3.5.0' } })
    writeLockfile('.claude/skills', { zod: { packageName: 'zod', version: '3.23.0' } })
    expect(packages(readProjectLock(cwd))).toEqual(['vue@3.5.0'])
  })
})
