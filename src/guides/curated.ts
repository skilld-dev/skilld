/**
 * Curated guide seed — the package set we generate migration guides for.
 *
 * Sourced from skilld's `REPO_REGISTRY` (the frameworks/libs we already track),
 * deduped to one headline package per repo so we don't emit guides for internal
 * sub-packages like `@vue/shared`. Expand the registry to grow the guide set.
 */

import { REPO_REGISTRY } from '../sources/package-registry.data.ts'

export interface CuratedListOptions {
  /** Emit every package, not just the primary one per repo. */
  all?: boolean
}

/** The headline package for a repo: the `primary` one, else the first listed. */
function headlinePackage(packages: Record<string, { primary?: boolean }>): string | undefined {
  const names = Object.keys(packages)
  return names.find(n => packages[n]?.primary) ?? names[0]
}

export function listCuratedPackages(opts: CuratedListOptions = {}): string[] {
  const out = new Set<string>()
  for (const entry of Object.values(REPO_REGISTRY)) {
    if (opts.all) {
      for (const name of Object.keys(entry.packages))
        out.add(name)
    }
    else {
      const headline = headlinePackage(entry.packages)
      if (headline)
        out.add(headline)
    }
  }
  return [...out]
}
