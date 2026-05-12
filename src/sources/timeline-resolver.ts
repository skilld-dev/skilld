/**
 * Timeline-references cascade: GitHub issues, discussions, and releases.
 *
 * Distinct from `resolveContentDocs` (which handles primary docs). Each
 * timeline source has its own existsSync guard, so this module owns its cache
 * writes (idempotent — won't refetch if data exists).
 *
 * Repo-level data lives at `~/.skilld/references/<owner>/<repo>/{issues,
 * discussions,releases}/` when a GitHub repo is known; otherwise it falls
 * back to the per-package cache.
 */

import type { FeaturesConfig } from '../core/config.ts'
import type { IndexDoc } from './content-resolver.ts'
import type { ResolvedPackage } from './types.ts'
import { existsSync } from 'node:fs'
import { join } from 'pathe'
import { getCacheDir, getRepoCacheDir } from '../cache/index.ts'
import { writeToCache, writeToRepoCache } from '../cache/internal/storage.ts'
import { parseFrontmatter } from '../core/markdown.ts'
import { sanitizeMarkdown } from '../core/sanitize.ts'
import { parseGitHubUrl } from '../core/url.ts'
import {
  fetchBlogReleases,
  fetchGitHubDiscussions,
  fetchGitHubIssues,
  fetchReleaseNotes,
  formatDiscussionAsMarkdown,
  formatIssueAsMarkdown,
  generateDiscussionIndex,
  generateIssueIndex,
  generateReleaseIndex,
  getBlogPreset,
  getPrereleaseChangelogRef,
  isGhAvailable,
  isPrerelease,
} from './index.ts'

export interface TimelineReferences {
  docsToIndex: IndexDoc[]
  hasIssues: boolean
  hasDiscussions: boolean
  hasReleases: boolean
  repoInfo?: { owner: string, repo: string }
}

export interface ResolveTimelineOptions {
  packageName: string
  resolved: ResolvedPackage
  version: string
  features: FeaturesConfig
  /** Lower-bound date for release/issue/discussion collection (ISO date) */
  from?: string
  onProgress: (message: string) => void
}

export async function resolveTimelineReferences(opts: ResolveTimelineOptions): Promise<TimelineReferences> {
  const { packageName, resolved, version, features, from, onProgress } = opts
  const docsToIndex: IndexDoc[] = []

  const gh = resolved.repoUrl ? parseGitHubUrl(resolved.repoUrl) : null
  const repoInfo = gh ? { owner: gh.owner, repo: gh.repo } : undefined

  const repoCacheDir = repoInfo ? getRepoCacheDir(repoInfo.owner, repoInfo.repo) : null
  const cacheDir = getCacheDir(packageName, version)
  const issuesDir = repoCacheDir ? join(repoCacheDir, 'issues') : join(cacheDir, 'issues')
  const discussionsDir = repoCacheDir ? join(repoCacheDir, 'discussions') : join(cacheDir, 'discussions')
  const releasesPath = repoCacheDir ? join(repoCacheDir, 'releases') : join(cacheDir, 'releases')

  // Issues
  if (features.issues && gh && isGhAvailable() && !existsSync(issuesDir)) {
    onProgress('Fetching issues via GitHub API')
    const issues = await fetchGitHubIssues(gh.owner, gh.repo, 30, resolved.releasedAt, from).catch(() => [])
    if (issues.length > 0) {
      onProgress(`Caching ${issues.length} issues`)
      const issueDocs = [
        ...issues.map(issue => ({
          path: `issues/issue-${issue.number}.md`,
          content: formatIssueAsMarkdown(issue),
        })),
        {
          path: 'issues/_INDEX.md',
          content: generateIssueIndex(issues),
        },
      ]
      if (repoInfo)
        writeToRepoCache(repoInfo.owner, repoInfo.repo, issueDocs)
      else
        writeToCache(packageName, version, issueDocs)
      for (const issue of issues) {
        docsToIndex.push({
          id: `issue-${issue.number}`,
          content: sanitizeMarkdown(`#${issue.number}: ${issue.title}\n\n${issue.body || ''}`),
          metadata: { package: packageName, source: `issues/issue-${issue.number}.md`, type: 'issue', number: issue.number },
        })
      }
    }
  }

  // Discussions
  if (features.discussions && gh && isGhAvailable() && !existsSync(discussionsDir)) {
    onProgress('Fetching discussions via GitHub API')
    const discussions = await fetchGitHubDiscussions(gh.owner, gh.repo, 20, resolved.releasedAt, from).catch(() => [])
    if (discussions.length > 0) {
      onProgress(`Caching ${discussions.length} discussions`)
      const discussionDocs = [
        ...discussions.map(d => ({
          path: `discussions/discussion-${d.number}.md`,
          content: formatDiscussionAsMarkdown(d),
        })),
        {
          path: 'discussions/_INDEX.md',
          content: generateDiscussionIndex(discussions),
        },
      ]
      if (repoInfo)
        writeToRepoCache(repoInfo.owner, repoInfo.repo, discussionDocs)
      else
        writeToCache(packageName, version, discussionDocs)
      for (const d of discussions) {
        docsToIndex.push({
          id: `discussion-${d.number}`,
          content: sanitizeMarkdown(`#${d.number}: ${d.title}\n\n${d.body || ''}`),
          metadata: { package: packageName, source: `discussions/discussion-${d.number}.md`, type: 'discussion', number: d.number },
        })
      }
    }
  }

  // Releases (GitHub releases + blog releases + CHANGELOG into a unified releases/ dir)
  if (features.releases && gh && isGhAvailable() && !existsSync(releasesPath)) {
    onProgress('Fetching releases via GitHub API')
    const changelogRef = isPrerelease(version) ? getPrereleaseChangelogRef(packageName) : undefined
    const releaseDocs = await fetchReleaseNotes(gh.owner, gh.repo, version, resolved.gitRef, packageName, from, changelogRef).catch(() => [])

    let blogDocs: Array<{ path: string, content: string }> = []
    if (getBlogPreset(packageName)) {
      onProgress('Fetching blog release notes')
      blogDocs = await fetchBlogReleases(packageName, version).catch(() => [])
    }

    const allDocs = [...releaseDocs, ...blogDocs]

    const blogEntries = blogDocs
      .filter(d => !d.path.endsWith('_INDEX.md'))
      .map((d) => {
        const versionMatch = d.path.match(/blog-(.+)\.md$/)
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

    if (allDocs.length > 0) {
      onProgress(`Caching ${allDocs.length} releases`)
      if (repoInfo)
        writeToRepoCache(repoInfo.owner, repoInfo.repo, allDocs)
      else
        writeToCache(packageName, version, allDocs)
      for (const doc of allDocs) {
        docsToIndex.push({
          id: doc.path,
          content: doc.content,
          metadata: { package: packageName, source: doc.path, type: 'release' },
        })
      }
    }
  }

  return {
    docsToIndex,
    hasIssues: features.issues && existsSync(issuesDir),
    hasDiscussions: features.discussions && existsSync(discussionsDir),
    hasReleases: features.releases && existsSync(releasesPath),
    repoInfo,
  }
}
