/**
 * npm metadata resolver — bootstrap step of the cascade.
 *
 * Fetches the package's npm registry record. On success, seeds
 * `ctx.result` with name/version/description/deps and records repo +
 * homepage hints for downstream resolvers. On miss, fatal-exits the cascade.
 */

import type { ResolvedPackage } from '../types.ts'
import { isGitHubRepoUrl, isUselessDocsUrl, normalizeRepoUrl, parseGitHubUrl } from '../../core/url.ts'
import { fetchNpmPackage, fetchNpmRegistryMeta } from '../npm-registry.ts'
import { defineResolver } from '../resolver-registry.ts'

export const npmResolver = defineResolver({
  id: 'npm',
  async run(ctx) {
    ctx.options.onProgress?.('npm')
    const pkg = await fetchNpmPackage(ctx.packageName)
    if (!pkg) {
      ctx.attempts.push({
        source: 'npm',
        url: `https://registry.npmjs.org/${ctx.packageName}/latest`,
        status: 'not-found',
        message: 'Package not found on npm registry',
      })
      return { kind: 'fatal' }
    }

    ctx.attempts.push({
      source: 'npm',
      url: `https://registry.npmjs.org/${ctx.packageName}/latest`,
      status: 'success',
      message: `Found ${pkg.name}@${pkg.version}`,
    })

    const registryMeta = pkg.version
      ? await fetchNpmRegistryMeta(ctx.packageName, pkg.version)
      : {}

    const result: ResolvedPackage = {
      name: pkg.name,
      version: pkg.version,
      releasedAt: registryMeta.releasedAt,
      description: pkg.description,
      dependencies: pkg.dependencies,
      distTags: registryMeta.distTags,
    }

    if (typeof pkg.repository === 'object' && pkg.repository?.url) {
      ctx.rawRepoUrl = pkg.repository.url
      const normalized = normalizeRepoUrl(pkg.repository.url)
      if (!normalized.includes('://') && normalized.includes('/') && !normalized.includes(':'))
        result.repoUrl = `https://github.com/${normalized}`
      else
        result.repoUrl = normalized
      ctx.subdir = pkg.repository.directory
    }
    else if (typeof pkg.repository === 'string') {
      if (pkg.repository.includes('://')) {
        const gh = parseGitHubUrl(pkg.repository)
        if (gh)
          result.repoUrl = `https://github.com/${gh.owner}/${gh.repo}`
      }
      else {
        const repo = pkg.repository.replace(/^github:/, '')
        if (repo.includes('/') && !repo.includes(':'))
          result.repoUrl = `https://github.com/${repo}`
      }
    }

    if (pkg.homepage && !isGitHubRepoUrl(pkg.homepage) && !isUselessDocsUrl(pkg.homepage))
      result.docsUrl = pkg.homepage

    ctx.npm = pkg
    ctx.result = result
    return { kind: 'ok' }
  },
})
