/**
 * Releases step: GitHub releases + curated blog releases + CHANGELOG, unified
 * into a single `releases/` cache dir.
 */

import type { StepResolver } from '../cascade.ts'
import type { TimelineCtx } from './types.ts'
import { existsSync } from 'node:fs'
import { writeToRepoCache } from '../../../cache/internal/storage.ts'
import { parseFrontmatter } from '../../../core/markdown.ts'
import { fetchBlogReleases } from '../../blog-releases.ts'
import { isGhAvailable } from '../../issues.ts'
import { getBlogPreset, getPrereleaseChangelogRef } from '../../package-registry.ts'
import { fetchReleaseNotes, generateReleaseIndex, isPrerelease } from '../../releases.ts'
import { defineStep } from '../cascade.ts'

const BLOG_VERSION_RE = /blog-(.+)\.md$/

export const releasesStep: StepResolver<TimelineCtx> = defineStep<TimelineCtx>({
  id: 'releases',
  canResolve: ctx => ctx.features.releases && !!ctx.repoInfo && isGhAvailable() && !existsSync(ctx.releasesPath),
  async run(ctx) {
    const { owner, repo } = ctx.repoInfo!
    const { packageName, version, resolved, from } = ctx

    ctx.onProgress('Fetching releases via GitHub API')
    const changelogRef = isPrerelease(version) ? getPrereleaseChangelogRef(packageName) : undefined
    const releaseDocs = await fetchReleaseNotes(owner, repo, version, resolved.gitRef, packageName, from, changelogRef).catch(() => [])

    let blogDocs: Array<{ path: string, content: string }> = []
    if (getBlogPreset(packageName)) {
      ctx.onProgress('Fetching blog release notes')
      blogDocs = await fetchBlogReleases(packageName, version).catch(() => [])
    }

    const allDocs = [...releaseDocs, ...blogDocs]

    const blogEntries = blogDocs
      .filter(d => !d.path.endsWith('_INDEX.md'))
      .map((d) => {
        const versionMatch = d.path.match(BLOG_VERSION_RE)
        const fm = parseFrontmatter(d.content)
        return {
          version: versionMatch?.[1] ?? '',
          title: fm.title ?? `Release ${versionMatch?.[1]}`,
          date: fm.date ?? '',
        }
      })
      .filter(b => b.version)

    const ghReleases = releaseDocs
      .filter(d => d.path.startsWith('releases/') && !d.path.endsWith('CHANGELOG.md'))
      .map((d) => {
        const fm = parseFrontmatter(d.content)
        const tag = fm.tag ?? ''
        const name = fm.name ?? tag
        const published = fm.published ?? ''
        return { id: 0, tag, name, prerelease: false, createdAt: published, publishedAt: published, markdown: '' }
      })
      .filter(r => r.tag)

    const hasChangelog = allDocs.some(d => d.path === 'releases/CHANGELOG.md')

    if (ghReleases.length > 0 || blogEntries.length > 0) {
      allDocs.push({
        path: 'releases/_INDEX.md',
        content: generateReleaseIndex({ releases: ghReleases, packageName, blogReleases: blogEntries, hasChangelog }),
      })
    }

    if (allDocs.length === 0)
      return

    ctx.onProgress(`Caching ${allDocs.length} releases`)
    writeToRepoCache(owner, repo, allDocs)
    for (const doc of allDocs) {
      ctx.docsToIndex.push({
        id: doc.path,
        content: doc.content,
        metadata: { package: packageName, source: doc.path, type: 'release' },
      })
    }
  },
})
