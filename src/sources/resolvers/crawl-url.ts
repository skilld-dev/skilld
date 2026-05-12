/**
 * Crawl URL resolver — populates `result.crawlUrl` from package-registry.
 *
 * Pure registry lookup; no network. Some packages (e.g. motion-v) have a
 * curated crawl pattern used later by the content stage.
 */

import { getCrawlUrl } from '../package-registry.ts'
import { defineResolver } from '../resolver-registry.ts'

export const crawlUrlResolver = defineResolver({
  id: 'crawl',
  canResolve: ctx => !!ctx.result,
  async run(ctx) {
    const crawlUrl = getCrawlUrl(ctx.packageName)
    if (crawlUrl)
      ctx.result!.crawlUrl = crawlUrl
    return { kind: 'ok' }
  },
})
