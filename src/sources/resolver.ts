/**
 * URL resolution cascade entry points.
 *
 * Thin conveniences over `createContentResolver(defaultResolvers).resolve`.
 * Walks npm → github-search → github-docs → github-meta → readme → crawl
 * → llms.txt → local. See `./resolvers/default.ts` for the canonical order.
 */

import type { ResolveOptions, ResolveStep } from './resolver-registry.ts'
import type { ResolvedPackage, ResolveResult } from './types.ts'
import { createContentResolver } from './resolver-registry.ts'
import { defaultResolvers } from './resolvers/default.ts'

export type { ResolveOptions, ResolveStep }

const defaultContentResolver = createContentResolver({ resolvers: defaultResolvers })

export async function resolvePackageDocs(packageName: string, options: ResolveOptions = {}): Promise<ResolvedPackage | null> {
  const result = await defaultContentResolver.resolve(packageName, options)
  return result.package
}

export async function resolvePackageDocsWithAttempts(packageName: string, options: ResolveOptions = {}): Promise<ResolveResult> {
  return defaultContentResolver.resolve(packageName, options)
}
