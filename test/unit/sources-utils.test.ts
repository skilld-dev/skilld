import { describe, expect, it } from 'vitest'
import { isGitHubRepoUrl, normalizeRepoUrl, parseGitHubRepoSlug, parseGitHubUrl, parsePackageSpec } from '../../src/core/url'

describe('sources/utils', () => {
  describe('isGitHubRepoUrl', () => {
    it('returns true for github.com URLs', () => {
      expect(isGitHubRepoUrl('https://github.com/vuejs/vue')).toBe(true)
      expect(isGitHubRepoUrl('https://www.github.com/vuejs/vue')).toBe(true)
    })

    it('returns false for non-GitHub URLs', () => {
      expect(isGitHubRepoUrl('https://vuejs.org')).toBe(false)
      expect(isGitHubRepoUrl('https://gitlab.com/repo')).toBe(false)
    })

    it('handles invalid URLs gracefully', () => {
      expect(isGitHubRepoUrl('not-a-url')).toBe(false)
      expect(isGitHubRepoUrl('')).toBe(false)
    })
  })

  describe('parseGitHubUrl', () => {
    it('extracts owner and repo', () => {
      expect(parseGitHubUrl('https://github.com/vuejs/vue')).toEqual({
        owner: 'vuejs',
        repo: 'vue',
      })
      expect(parseGitHubUrl('https://github.com/nuxt/nuxt')).toEqual({
        owner: 'nuxt',
        repo: 'nuxt',
      })
    })

    it('handles URLs with extra path segments', () => {
      expect(parseGitHubUrl('https://github.com/owner/repo/tree/main')).toEqual({
        owner: 'owner',
        repo: 'repo',
      })
    })

    it('strips .git suffix from repo name', () => {
      expect(parseGitHubUrl('https://github.com/nextauthjs/next-auth.git')).toEqual({
        owner: 'nextauthjs',
        repo: 'next-auth',
      })
      expect(parseGitHubUrl('git+https://github.com/owner/repo.git')).toEqual({
        owner: 'owner',
        repo: 'repo',
      })
    })

    it('returns null for invalid URLs', () => {
      expect(parseGitHubUrl('https://gitlab.com/owner/repo')).toBeNull()
      expect(parseGitHubUrl('not-a-url')).toBeNull()
    })
  })

  describe('parseGitHubRepoSlug', () => {
    it('extracts owner/repo slug', () => {
      expect(parseGitHubRepoSlug('https://github.com/vuejs/vue.git#main')).toBe('vuejs/vue')
    })

    it('returns undefined for missing or non-GitHub URLs', () => {
      expect(parseGitHubRepoSlug(undefined)).toBeUndefined()
      expect(parseGitHubRepoSlug('https://example.com/foo/bar')).toBeUndefined()
    })
  })

  describe('parsePackageSpec', () => {
    it('parses plain package name', () => {
      expect(parsePackageSpec('vue')).toEqual({ name: 'vue' })
    })

    it('parses unscoped package with dist-tag', () => {
      expect(parsePackageSpec('vue@beta')).toEqual({ name: 'vue', tag: 'beta' })
      expect(parsePackageSpec('vue@latest')).toEqual({ name: 'vue', tag: 'latest' })
    })

    it('parses unscoped package with version', () => {
      expect(parsePackageSpec('vue@3.5.0')).toEqual({ name: 'vue', tag: '3.5.0' })
    })

    it('parses scoped package without tag', () => {
      expect(parsePackageSpec('@vue/reactivity')).toEqual({ name: '@vue/reactivity' })
    })

    it('parses scoped package with dist-tag', () => {
      expect(parsePackageSpec('@vue/reactivity@beta')).toEqual({ name: '@vue/reactivity', tag: 'beta' })
    })

    it('parses scoped package with version', () => {
      expect(parsePackageSpec('@nuxt/kit@3.15.0')).toEqual({ name: '@nuxt/kit', tag: '3.15.0' })
    })
  })

  describe('normalizeRepoUrl', () => {
    it('removes git+ prefix', () => {
      expect(normalizeRepoUrl('git+https://github.com/owner/repo.git'))
        .toBe('https://github.com/owner/repo')
    })

    it('removes .git suffix', () => {
      expect(normalizeRepoUrl('https://github.com/owner/repo.git'))
        .toBe('https://github.com/owner/repo')
    })

    it('converts git:// to https://', () => {
      expect(normalizeRepoUrl('git://github.com/owner/repo'))
        .toBe('https://github.com/owner/repo')
    })

    it('converts ssh URLs', () => {
      expect(normalizeRepoUrl('ssh://git@github.com/owner/repo'))
        .toBe('https://github.com/owner/repo')
    })

    it('handles already normalized URLs', () => {
      expect(normalizeRepoUrl('https://github.com/owner/repo'))
        .toBe('https://github.com/owner/repo')
    })
  })
})
