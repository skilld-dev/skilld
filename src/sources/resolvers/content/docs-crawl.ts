/**
 * docsUrl fallback crawl: when no actual doc files have been committed yet,
 * crawl the package's documentation site via its sitemap / docsUrl pattern.
 */

import type { StepResolver } from '../cascade.ts'
import type { ContentCtx } from './types.ts'
import { fetchCrawledDocs, toCrawlPattern } from '../../crawl.ts'
import { defineStep } from '../cascade.ts'

export const docsCrawlStep: StepResolver<ContentCtx> = defineStep<ContentCtx>({
  id: 'docs-crawl',
  canResolve: ctx => !!ctx.resolved.docsUrl && !ctx.docs.some(d => d.path.startsWith('docs/')),
  async run(ctx) {
    const crawlPattern = ctx.resolved.crawlUrl || toCrawlPattern(ctx.resolved.docsUrl!)
    ctx.onProgress('Crawling docs site')
    const maxPages = ctx.resolved.crawlUrl ? 200 : 400
    const crawled = await fetchCrawledDocs(crawlPattern, ctx.onProgress, maxPages).catch((err) => {
      ctx.warnings.push(`Crawl failed for ${crawlPattern}: ${err?.message || err}`)
      return []
    })
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
      ctx.docSource = crawlPattern
      ctx.docsType = 'docs'
    }
  },
})
