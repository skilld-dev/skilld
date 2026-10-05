import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { detectBehaviors, MAX_BEHAVIOR_LOCATIONS } from '../src/behaviors.ts'

interface Case {
  name: string
  files: Array<{ path: string, text: string, executable?: boolean }>
  behaviors: Record<string, string[]>
}

// The same corpus the Rust matcher runs, so both report identical behaviors.
const { cases } = JSON.parse(readFileSync(
  new URL('../../../contracts/fixtures/skill-behaviors/cases.json', import.meta.url),
  'utf8',
)) as { cases: Case[] }

describe('detectBehaviors', () => {
  it.each(cases.map(testCase => [testCase.name, testCase] as const))('%s', (_, testCase) => {
    const actual = Object.fromEntries(detectBehaviors(testCase.files).map(behavior => [
      behavior.id,
      behavior.locations.map(location => location.line === null ? location.path : `${location.path}:${location.line}`),
    ]))
    expect(actual).toEqual(testCase.behaviors)
  })

  it('keeps the first locations and counts every matching line', () => {
    const text = `\`\`\`sh\n${'sudo true\n'.repeat(8)}\`\`\`\n`
    const privilege = detectBehaviors([{ path: 'SKILL.md', text }]).find(behavior => behavior.id === 'privilege')
    expect(privilege?.total).toBe(8)
    expect(privilege?.locations).toHaveLength(MAX_BEHAVIOR_LOCATIONS)
  })

  it('applies file rules to a file whose text it never read', () => {
    expect(detectBehaviors([{ path: 'scripts/setup.sh' }])).toEqual([{
      id: 'scripts',
      tier: 'show',
      label: 'Ships scripts',
      locations: [{ path: 'scripts/setup.sh', line: null }],
      total: 1,
    }])
  })
})
