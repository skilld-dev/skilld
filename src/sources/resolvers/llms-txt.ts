/**
 * llms.txt resolver — discovers `llms.txt` under the package's docs site.
 *
 * Two phases:
 *   1. Look for an llms.txt URL anchored at `result.docsUrl` and record it.
 *   2. If we also have git-docs, cross-validate the heuristic git-docs
 *      against the llms.txt link set; discard git-docs if match ratio is low.
 */

import { validateGitDocsWithLlms } from '../github.ts'
import { fetchLlmsTxt, fetchLlmsUrl } from '../llms.ts'
import { defineResolver } from '../resolver-registry.ts'

export const llmsTxtResolver = defineResolver({
  id: 'llms.txt',
  canResolve: ctx => !!ctx.result?.docsUrl,
  async run(ctx) {
    const result = ctx.result!
    ctx.options.onProgress?.('llms.txt')
    const llmsUrl = await fetchLlmsUrl(result.docsUrl!)
    if (llmsUrl) {
      result.llmsUrl = llmsUrl
      ctx.attempts.push({ source: 'llms.txt', url: llmsUrl, status: 'success' })
    }
    else {
      ctx.attempts.push({
        source: 'llms.txt',
        url: `${new URL(result.docsUrl!).origin}/llms.txt`,
        status: 'not-found',
        message: 'No llms.txt at docs URL',
      })
    }

    // Cross-validate heuristic git-docs against llms.txt link set.
    if (result.gitDocsUrl && result.llmsUrl && ctx.gitDocsAllFiles) {
      const llmsContent = await fetchLlmsTxt(result.llmsUrl)
      if (llmsContent && llmsContent.links.length > 0) {
        const validation = validateGitDocsWithLlms(llmsContent.links, ctx.gitDocsAllFiles)
        if (!validation.isValid) {
          ctx.attempts.push({
            source: 'github-docs',
            url: result.gitDocsUrl,
            status: 'not-found',
            message: `Heuristic git docs don't match llms.txt links (${Math.round(validation.matchRatio * 100)}% match), preferring llms.txt`,
          })
          result.gitDocsUrl = undefined
          result.gitRef = undefined
        }
      }
    }

    return { kind: 'ok' }
  },
})
