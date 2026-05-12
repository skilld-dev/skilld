import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

describe('install docs restore', () => {
  it('uses the shared content resolver instead of duplicating the docs cascade', () => {
    const source = readFileSync('src/commands/install.ts', 'utf8')

    expect(source).toContain('fetchAndCacheResources')
    expect(source).not.toMatch(/\b(fetchGitDocs|fetchGitHubRaw|fetchLlmsTxt|downloadLlmsDocs|fetchReadmeContent|normalizeLlmsLinks|filterFrameworkDocs|isShallowGitDocs|resolveContentDocs)\b/)
  })
})
