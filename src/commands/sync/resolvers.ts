/**
 * `PackageResolver` implementations: turn a spec into a `ResolvedSpec` (or
 * `UnresolvedSpec` with attempts / shipped fallback).
 */

import type { AgentType } from '../../agent/index.ts'
import type { ResolverOpts, ResolverResult } from './phases.ts'
import { handleShippedSkills } from '../../agent/skill-installer.ts'
import { resolveGitHubRepo } from '../../sources/github.ts'
import { resolvePackageOrCrate } from '../../sources/index.ts'

export type PackageResolver = (spec: string, opts: ResolverOpts) => Promise<ResolverResult>

export const npmResolver: PackageResolver = async (spec, opts) => {
  const resolution = await resolvePackageOrCrate(spec, {
    cwd: opts.cwd,
    onProgress: msg => opts.onProgress(`${spec}: ${msg}`),
  })
  const { isCrate, packageName, identityPackageName, storagePackageName, requestedTag, localVersion, attempts, registryVersion } = resolution

  if (!resolution.resolved) {
    const result: ResolverResult = {
      identityName: identityPackageName,
      attempts,
      registryVersion,
    }
    if (!isCrate) {
      const shippedVersion = localVersion || registryVersion || 'latest'
      const shipped = handleShippedSkills(packageName, shippedVersion, opts.cwd, opts.agent, opts.global)
      if (shipped)
        result.shipped = shipped.shipped
    }
    return result
  }

  const resolved = resolution.resolved
  const version = isCrate
    ? (resolved.version || requestedTag || 'latest')
    : (localVersion || resolved.version || 'latest')

  return {
    identityName: identityPackageName,
    storageName: storagePackageName,
    version,
    resolved,
    kind: isCrate ? 'crate' : 'npm',
    requestedTag,
    localVersion,
  }
}

export function createGithubResolver(owner: string, repo: string): PackageResolver {
  return async (_spec, opts) => {
    const resolved = await resolveGitHubRepo(owner, repo, msg => opts.onProgress(msg))
    if (!resolved) {
      return {
        identityName: `${owner}-${repo}`,
        attempts: [{ source: 'github-meta', status: 'not-found', message: `Could not find docs for ${owner}/${repo}` }],
      }
    }
    const repoUrl = `https://github.com/${owner}/${repo}`
    const name = `${owner}-${repo}`
    return {
      identityName: name,
      storageName: name,
      version: resolved.version || 'main',
      resolved: { ...resolved, repoUrl },
      kind: 'github',
    }
  }
}

export type { AgentType }
