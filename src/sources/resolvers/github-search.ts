/**
 * GitHub search fallback — runs only when npm metadata had no repository URL.
 *
 * Searches GitHub by package name and records the discovered repo URL on
 * `ctx.result.repoUrl` so downstream github-* resolvers can pick it up.
 */

import { searchGitHubRepo } from '../github.ts'
import { defineResolver } from '../resolver-registry.ts'

export const githubSearchResolver = defineResolver({
  id: 'github-search',
  canResolve: ctx => !!ctx.result && !ctx.result.repoUrl,
  async run(ctx) {
    const result = ctx.result!
    ctx.options.onProgress?.('github-search')
    const searchedUrl = await searchGitHubRepo(ctx.packageName)
    if (searchedUrl) {
      result.repoUrl = searchedUrl
      ctx.attempts.push({
        source: 'github-search',
        url: searchedUrl,
        status: 'success',
        message: `Found via GitHub search: ${searchedUrl}`,
      })
      return { kind: 'ok' }
    }
    ctx.attempts.push({
      source: 'github-search',
      status: 'not-found',
      message: 'No repository URL in package.json and GitHub search found no match',
    })
    return { kind: 'skip' }
  },
})
