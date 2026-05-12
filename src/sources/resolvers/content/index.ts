/**
 * Default content cascade order. Order is load-bearing.
 *
 *   1. git-docs     — versioned docs/** at the package's git tag
 *   2. crawl-url    — registry-configured crawl pattern (e.g. motion-v)
 *   3. llms-txt     — package's llms.txt + linked docs
 *   4. docs-crawl   — sitemap-driven crawl of docsUrl
 *   5. readme       — README fallback
 */

import type { StepResolver } from '../cascade.ts'
import type { ContentCtx } from './types.ts'
import { crawlUrlStep } from './crawl-url.ts'
import { docsCrawlStep } from './docs-crawl.ts'
import { gitDocsStep } from './git-docs.ts'
import { llmsTxtStep } from './llms-txt.ts'
import { readmeStep } from './readme.ts'

export type { ContentCtx } from './types.ts'

export const defaultContentSteps: ReadonlyArray<StepResolver<ContentCtx>> = [
  gitDocsStep,
  crawlUrlStep,
  llmsTxtStep,
  docsCrawlStep,
  readmeStep,
]
