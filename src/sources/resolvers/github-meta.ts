/**
 * GitHub repo metadata resolver — fills `docsUrl` from the repo's homepage.
 *
 * Only runs when we have a GitHub repo URL and don't yet have a docsUrl
 * (set earlier from npm `homepage`).
 */

import { isUselessDocsUrl, parseGitHubUrl } from '../../core/url.ts'
import { fetchGitHubRepoMeta } from '../github.ts'
import { defineResolver } from '../resolver-registry.ts'

export const githubMetaResolver = defineResolver({
  id: 'github-meta',
  canResolve: ctx => !!ctx.result?.repoUrl?.includes('github.com') && !ctx.result.docsUrl,
  async run(ctx) {
    const result = ctx.result!
    const gh = parseGitHubUrl(result.repoUrl!)
    if (!gh)
      return { kind: 'skip' }

    ctx.options.onProgress?.('github-meta')
    const repoMeta = await fetchGitHubRepoMeta(gh.owner, gh.repo, ctx.packageName)
    if (repoMeta?.homepage && !isUselessDocsUrl(repoMeta.homepage)) {
      result.docsUrl = repoMeta.homepage
      ctx.attempts.push({
        source: 'github-meta',
        url: result.repoUrl!,
        status: 'success',
        message: `Found homepage: ${repoMeta.homepage}`,
      })
      return { kind: 'ok' }
    }
    ctx.attempts.push({
      source: 'github-meta',
      url: result.repoUrl!,
      status: 'not-found',
      message: 'No homepage in repo metadata',
    })
    return { kind: 'skip' }
  },
})
