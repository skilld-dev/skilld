/**
 * llms.txt step: fetch the package's `llms.txt`, download linked docs.
 * Runs only when no primary docs are committed yet.
 */

import type { StepResolver } from '../cascade.ts'
import type { ContentCtx } from './types.ts'
import { join } from 'pathe'
import { downloadLlmsDocs, fetchLlmsTxt, normalizeLlmsLinks } from '../../llms.ts'
import { defineStep } from '../cascade.ts'

export const llmsTxtStep: StepResolver<ContentCtx> = defineStep<ContentCtx>({
  id: 'llms.txt',
  canResolve: ctx => !!ctx.resolved.llmsUrl && ctx.docs.length === 0,
  async run(ctx) {
    const llmsUrl = ctx.resolved.llmsUrl!
    ctx.onProgress('Fetching llms.txt')
    const llmsContent = await fetchLlmsTxt(llmsUrl)
    if (!llmsContent)
      return

    ctx.docSource = llmsUrl
    ctx.docsType = 'llms.txt'
    const baseUrl = ctx.resolved.docsUrl || new URL(llmsUrl).origin
    ctx.docs.push({ path: 'llms.txt', content: normalizeLlmsLinks(llmsContent.raw, baseUrl) })

    if (llmsContent.links.length > 0) {
      ctx.onProgress(`Downloading ${llmsContent.links.length} linked docs`)
      const linked = await downloadLlmsDocs(llmsContent, baseUrl, (_url, done, total) => {
        ctx.onProgress(`Downloading linked doc ${done + 1}/${total}`)
      })
      for (const doc of linked) {
        if (!ctx.isFrameworkDoc(doc.url))
          continue
        const localPath = doc.url.startsWith('/') ? doc.url.slice(1) : doc.url
        const cachePath = join('docs', ...localPath.split('/'))
        ctx.docs.push({ path: cachePath, content: doc.content })
        ctx.docsToIndex.push({
          id: doc.url,
          content: doc.content,
          metadata: { package: ctx.packageName, source: cachePath, type: 'doc' },
        })
      }
      if (linked.length > 0)
        ctx.docsType = 'docs'
    }
  },
})
