/**
 * Typed registry for the URL resolution cascade.
 *
 * Each `Resolver` is one step in the cascade (npm metadata, github tags,
 * llms.txt, etc). The cascade is stateful: each step reads and mutates a
 * shared `ResolvedPackage` accumulator and pushes diagnostic attempts.
 *
 * `createContentResolver` walks an ordered list of resolvers and aggregates
 * results into a `ResolveResult`. Built-in resolvers live in
 * `./resolvers/` and the default order is `defaultResolvers`.
 */

import type { NpmPackageInfo, ResolveAttempt, ResolvedPackage, ResolveResult } from './types.ts'

export type ResolveStep
  = | 'npm'
    | 'github-docs'
    | 'github-meta'
    | 'github-search'
    | 'readme'
    | 'llms.txt'
    | 'crawl'
    | 'local'

/**
 * Mutable cascade state shared across resolvers.
 *
 * Resolvers read prior fields (e.g. `repoUrl`, `docsUrl`) to decide what to
 * do, and write their findings back so later resolvers can see them.
 */
export interface ResolveCtx {
  /** Package name being resolved. */
  packageName: string
  /** Caller-provided options. */
  options: ResolveOptions
  /** Accumulator — built up step by step. `null` until npm step succeeds. */
  result: ResolvedPackage | null
  /** Diagnostic trail. */
  attempts: ResolveAttempt[]
  /** npm metadata fetched by the npm resolver — read by later steps. */
  npm?: NpmPackageInfo
  /** Repository URL string from npm package.json (pre-normalization). */
  rawRepoUrl?: string
  /** Subdirectory inside the repo (npm `repository.directory`). */
  subdir?: string
  /** All files discovered under git docs/, used by llms.txt validation. */
  gitDocsAllFiles?: string[]
}

export interface ResolveOptions {
  /** Specific version to target for versioned git docs. */
  version?: string
  /** Working directory for local node_modules readme fallback. */
  cwd?: string
  /** Called before each cascade step runs. */
  onProgress?: (step: ResolveStep) => void
}

/** Outcome of a single resolver run. */
export type ResolverOutcome
  = | { kind: 'ok' }
    | { kind: 'skip', reason?: string }
    | { kind: 'fatal', registryVersion?: string }

export interface Resolver {
  id: ResolveStep | string
  /** Cheap pre-check; if false the resolver is skipped without invoking `run`. */
  canResolve?: (ctx: ResolveCtx) => boolean
  /** Mutates `ctx.result` / `ctx.attempts`; may short-circuit via `fatal`. */
  run: (ctx: ResolveCtx) => Promise<ResolverOutcome>
}

export function defineResolver(r: Resolver): Resolver {
  return r
}

export interface ContentResolver {
  /** Walks the configured resolvers, returning the aggregate cascade result. */
  resolve: (packageName: string, options?: ResolveOptions) => Promise<ResolveResult>
}

export function createContentResolver(opts: { resolvers: Resolver[] }): ContentResolver {
  return {
    async resolve(packageName, options = {}) {
      const ctx: ResolveCtx = {
        packageName,
        options,
        result: null,
        attempts: [],
      }

      let registryVersion: string | undefined

      for (const resolver of opts.resolvers) {
        if (resolver.canResolve && !resolver.canResolve(ctx))
          continue
        const outcome = await resolver.run(ctx)
        if (outcome.kind === 'fatal') {
          return {
            package: null,
            attempts: ctx.attempts,
            registryVersion: outcome.registryVersion ?? registryVersion,
          }
        }
      }

      // Capture npm version even when downstream resolvers find nothing.
      registryVersion = ctx.npm?.version

      // If no useful URLs found, return null package but keep attempts.
      const r = ctx.result
      if (!r || (!r.docsUrl && !r.llmsUrl && !r.readmeUrl && !r.gitDocsUrl))
        return { package: null, attempts: ctx.attempts, registryVersion }

      return { package: r, attempts: ctx.attempts, registryVersion }
    },
  }
}
