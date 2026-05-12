/**
 * GitHub README resolver — fetches the readme URL at the resolved git ref.
 *
 * Runs whenever we have a GitHub repo URL. Uses any prior `gitRef` from
 * the git-tag step and the npm `repository.directory` subdir if present.
 */

import { parseGitHubUrl } from '../../core/url.ts'
import { fetchReadme } from '../github.ts'
import { defineResolver } from '../resolver-registry.ts'

export const githubReadmeResolver = defineResolver({
  id: 'readme',
  canResolve: ctx => !!ctx.result?.repoUrl?.includes('github.com'),
  async run(ctx) {
    const result = ctx.result!
    const gh = parseGitHubUrl(result.repoUrl!)
    if (!gh)
      return { kind: 'skip' }

    ctx.options.onProgress?.('readme')
    const readmeUrl = await fetchReadme(gh.owner, gh.repo, ctx.subdir, result.gitRef)
    if (readmeUrl) {
      result.readmeUrl = readmeUrl
      ctx.attempts.push({ source: 'readme', url: readmeUrl, status: 'success' })
      return { kind: 'ok' }
    }
    ctx.attempts.push({
      source: 'readme',
      url: `${result.repoUrl}/README.md`,
      status: 'not-found',
      message: 'No README found',
    })
    return { kind: 'skip' }
  },
})
