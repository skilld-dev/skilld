/**
 * Registry-configured crawl-URL step: fetch docs from a curated crawl pattern
 * (e.g. motion-v's website). Runs only when no primary docs are committed yet.
 */

import type { StepResolver } from '../cascade.ts'
import type { ContentCtx } from './types.ts'
import { fetchCrawledDocs } from '../../crawl.ts'
import { defineStep } from '../cascade.ts'

export const crawlUrlStep: StepResolver<ContentCtx> = defineStep<ContentCtx>({
  id: 'crawl-url',
  canResolve: ctx => !!ctx.resolved.crawlUrl && ctx.docs.length === 0,
  async run(ctx) {
    const crawlUrl = ctx.resolved.crawlUrl!
    ctx.onProgress('Crawling website')
    const crawled = await fetchCrawledDocs(crawlUrl, ctx.onProgress).catch((err) => {
      ctx.warnings.push(`Crawl failed for ${crawlUrl}: ${err?.message || err}`)
      return []
    })
    if (crawled.length === 0)
      ctx.warnings.push(`Crawl returned 0 docs from ${crawlUrl}`)

    let added = 0
    for (const doc of crawled) {
      if (!ctx.isFrameworkDoc(doc.path))
        continue
      ctx.docs.push(doc)
      ctx.docsToIndex.push({
        id: doc.path,
        content: doc.content,
        metadata: { package: ctx.packageName, source: doc.path, type: 'doc' },
      })
      added++
    }
    if (added > 0) {
      ctx.docSource = crawlUrl
      ctx.docsType = 'docs'
    }
  },
})
