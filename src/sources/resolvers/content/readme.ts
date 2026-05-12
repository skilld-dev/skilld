/**
 * README fallback: last-resort primary doc when nothing else committed.
 */

import type { StepResolver } from '../cascade.ts'
import type { ContentCtx } from './types.ts'
import { fetchReadmeContent } from '../../github.ts'
import { defineStep } from '../cascade.ts'

export const readmeStep: StepResolver<ContentCtx> = defineStep<ContentCtx>({
  id: 'readme',
  canResolve: ctx => !!ctx.resolved.readmeUrl && ctx.docs.length === 0,
  async run(ctx) {
    ctx.onProgress('Fetching README')
    const content = await fetchReadmeContent(ctx.resolved.readmeUrl!)
    if (!content)
      return
    ctx.docs.push({ path: 'docs/README.md', content })
    ctx.docsToIndex.push({
      id: 'README.md',
      content,
      metadata: { package: ctx.packageName, source: 'docs/README.md', type: 'doc' },
    })
  },
})
