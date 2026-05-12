import { describe, expect, it } from 'vitest'
import { isGitHubRepoUrl, normalizeRepoUrl, parseGitHubUrl } from './url.ts'

describe('isGitHubRepoUrl', () => {
  it('returns true for github.com URLs', () => {
    expect(isGitHubRepoUrl('https://github.com/user/repo')).toBe(true)
    expect(isGitHubRepoUrl('https://www.github.com/user/repo')).toBe(true)
  })

  it('returns false for non-github URLs', () => {
    expect(isGitHubRepoUrl('https://vuejs.org')).toBe(false)
    expect(isGitHubRepoUrl('https://nuxt.com')).toBe(false)
  })

  it('returns false for invalid URLs', () => {
    expect(isGitHubRepoUrl('not-a-url')).toBe(false)
  })
})

describe('parseGitHubUrl', () => {
  it('extracts owner and repo', () => {
    const result = parseGitHubUrl('https://github.com/vuejs/vue')
    expect(result).toEqual({ owner: 'vuejs', repo: 'vue' })
  })

  it('returns null for non-github URLs', () => {
    expect(parseGitHubUrl('https://example.com/foo/bar')).toBeNull()
  })
})

describe('normalizeRepoUrl', () => {
  it('handles git+ prefix', () => {
    expect(normalizeRepoUrl('git+https://github.com/user/repo.git'))
      .toBe('https://github.com/user/repo')
  })

  it('handles .git suffix', () => {
    expect(normalizeRepoUrl('https://github.com/user/repo.git'))
      .toBe('https://github.com/user/repo')
  })

  it('handles git:// protocol', () => {
    expect(normalizeRepoUrl('git://github.com/user/repo'))
      .toBe('https://github.com/user/repo')
  })

  it('handles ssh URLs', () => {
    expect(normalizeRepoUrl('ssh://git@github.com/user/repo'))
      .toBe('https://github.com/user/repo')
  })
})
