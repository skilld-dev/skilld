/**
 * Default URL-resolution cascade order.
 *
 * Order is load-bearing — it preserves the success-rate characteristics
 * of the pre-registry inlined cascade. Steps:
 *
 *   1. npm           — bootstrap; mandatory.
 *   2. github-search — fallback when npm had no repository URL.
 *   3. github-docs   — versioned `docs/` at git tag.
 *   4. github-meta   — homepage from repo metadata (if no docsUrl yet).
 *   5. readme        — README at resolved git ref.
 *   6. crawl         — record curated crawl pattern.
 *   7. llms.txt      — discover + cross-validate against git-docs.
 *   8. local         — node_modules readme fallback.
 */

import type { Resolver } from '../resolver-registry.ts'
import { crawlUrlResolver } from './crawl-url.ts'
import { gitTagResolver } from './git-tag.ts'
import { githubMetaResolver } from './github-meta.ts'
import { githubReadmeResolver } from './github-readme.ts'
import { githubSearchResolver } from './github-search.ts'
import { llmsTxtResolver } from './llms-txt.ts'
import { localReadmeResolver } from './local-readme.ts'
import { npmResolver } from './npm.ts'

export const defaultResolvers: Resolver[] = [
  npmResolver,
  githubSearchResolver,
  gitTagResolver,
  githubMetaResolver,
  githubReadmeResolver,
  crawlUrlResolver,
  llmsTxtResolver,
  localReadmeResolver,
]
