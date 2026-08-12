import { describe, expect, it } from 'vitest'
import { getShippedSkills, resolvePkgDir } from '../../src/core/prepare.ts'

// `getCacheDir` rejects malformed names to block path traversal. These callers
// probe for an optional cache hit, so a rejection is a miss, not a crash: an
// unparsed spec used to abort the whole sync with an uncaught error.
describe('package dir probing', () => {
  it.each(['npm:vue', 'gh:owner/repo', '../escape'])('returns null for %j', (name) => {
    expect(resolvePkgDir(name, process.cwd(), '1.0.0')).toBeNull()
  })

  it.each(['npm:vue', '../escape'])('reports no shipped skills for %j', (name) => {
    expect(getShippedSkills(name, process.cwd(), '1.0.0')).toEqual([])
  })
})
