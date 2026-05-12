/**
 * Versioned git docs resolver — fetches `docs/` at the package's git tag.
 *
 * Only runs when we have a GitHub repo URL. Honors a caller-supplied
 * version override (`options.version`) but falls back to the npm version.
 * Records `gitDocsUrl`, `gitRef`, `gitDocsFallback`, and stashes
 * `gitDocsAllFiles` on the context for later llms.txt cross-validation.
 */

import { parseGitHubUrl } from '../../core/url.ts'
import { fetchGitDocs } from '../github.ts'
import { defineResolver } from '../resolver-registry.ts'

export const gitTagResolver = defineResolver({
  id: 'github-docs',
  canResolve: ctx => !!ctx.result?.repoUrl?.includes('github.com'),
  async run(ctx) {
    const result = ctx.result!
    const gh = parseGitHubUrl(result.repoUrl!)
    if (!gh)
      return { kind: 'skip' }

    const targetVersion = ctx.options.version || ctx.npm?.version
    if (!targetVersion)
      return { kind: 'skip' }

    ctx.options.onProgress?.('github-docs')
    const gitDocs = await fetchGitDocs(gh.owner, gh.repo, targetVersion, ctx.packageName, ctx.rawRepoUrl)
    if (gitDocs) {
      result.gitDocsUrl = gitDocs.baseUrl
      result.gitRef = gitDocs.ref
      result.gitDocsFallback = gitDocs.fallback
      ctx.gitDocsAllFiles = gitDocs.allFiles
      ctx.attempts.push({
        source: 'github-docs',
        url: gitDocs.baseUrl,
        status: 'success',
        message: gitDocs.fallback
          ? `Found ${gitDocs.files.length} docs at ${gitDocs.ref} (no tag for v${targetVersion})`
          : `Found ${gitDocs.files.length} docs at ${gitDocs.ref}`,
      })
      return { kind: 'ok' }
    }
    ctx.attempts.push({
      source: 'github-docs',
      url: `${result.repoUrl}/tree/v${targetVersion}/docs`,
      status: 'not-found',
      message: 'No docs/ folder found at version tag',
    })
    return { kind: 'skip' }
  },
})
