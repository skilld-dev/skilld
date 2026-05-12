/**
 * Versioned git-docs step: fetch `docs/**` at the package's git tag.
 *
 * Two outcomes:
 *   - Enough docs found → commit as primary (ctx.docsType='docs'), and also
 *     cache the package's `llms.txt` + linked supplementary docs.
 *   - "Shallow" result with an `llms.txt` available → write nothing and let
 *     the llms.txt step take over. A warning records the fallback ref.
 */

import type { CachedDoc, IndexDoc } from '../../content-resolver.ts'
import type { StepResolver } from '../cascade.ts'
import type { ContentCtx } from './types.ts'
import { join } from 'pathe'
import { parseGitHubUrl } from '../../../core/url.ts'
import { fetchGitDocs, isShallowGitDocs } from '../../github.ts'
import { downloadLlmsDocs, fetchLlmsTxt, normalizeLlmsLinks } from '../../llms.ts'
import { fetchGitHubRaw } from '../../utils.ts'
import { defineStep } from '../cascade.ts'

const BATCH_SIZE = 20

export const gitDocsStep: StepResolver<ContentCtx> = defineStep<ContentCtx>({
  id: 'git-docs',
  canResolve: ctx => !!ctx.resolved.gitDocsUrl && !!ctx.resolved.repoUrl,
  async run(ctx) {
    const gh = parseGitHubUrl(ctx.resolved.repoUrl!)
    if (!gh)
      return

    ctx.onProgress('Fetching git docs')
    const gitDocs = await fetchGitDocs(gh.owner, gh.repo, ctx.version, ctx.packageName)
    if (!gitDocs || gitDocs.files.length === 0)
      return

    if (gitDocs.fallback)
      ctx.warnings.push(`Docs fetched from ${gitDocs.ref} branch (no tag found for v${ctx.version})`)

    const results: Array<{ file: string, content: string } | null> = []
    for (let i = 0; i < gitDocs.files.length; i += BATCH_SIZE) {
      const batch = gitDocs.files.slice(i, i + BATCH_SIZE)
      ctx.onProgress(`Downloading docs ${Math.min(i + BATCH_SIZE, gitDocs.files.length)}/${gitDocs.files.length} from ${gitDocs.ref}`)
      const batchResults = await Promise.all(
        batch.map(async (file) => {
          const url = `${gitDocs.baseUrl}/${file}`
          const content = await fetchGitHubRaw(url)
          return content ? { file, content } : null
        }),
      )
      results.push(...batchResults)
    }

    const docs: CachedDoc[] = []
    const docsToIndex: IndexDoc[] = []
    for (const r of results) {
      if (!r)
        continue
      const stripped = gitDocs.docsPrefix ? r.file.replace(gitDocs.docsPrefix, '') : r.file
      const cachePath = stripped.startsWith('docs/') ? stripped : `docs/${stripped}`
      docs.push({ path: cachePath, content: r.content })
      docsToIndex.push({
        id: cachePath,
        content: r.content,
        metadata: { package: ctx.packageName, source: cachePath, type: 'doc' },
      })
    }

    // Shallow git-docs: defer to llms.txt step (don't commit anything yet).
    if (isShallowGitDocs(docs.length) && ctx.resolved.llmsUrl) {
      ctx.onProgress(`Shallow git-docs (${docs.length} files), trying llms.txt`)
      return
    }

    ctx.docs.push(...docs)
    ctx.docsToIndex.push(...docsToIndex)
    ctx.docSource = `${ctx.resolved.repoUrl}/tree/${gitDocs.ref}/docs`
    ctx.docsType = 'docs'

    // Always cache llms.txt alongside good git-docs as supplementary reference.
    if (ctx.resolved.llmsUrl) {
      ctx.onProgress('Caching supplementary llms.txt')
      const llmsContent = await fetchLlmsTxt(ctx.resolved.llmsUrl)
      if (llmsContent) {
        const baseUrl = ctx.resolved.docsUrl || new URL(ctx.resolved.llmsUrl).origin
        ctx.docs.push({ path: 'llms.txt', content: normalizeLlmsLinks(llmsContent.raw, baseUrl) })
        if (llmsContent.links.length > 0) {
          ctx.onProgress(`Downloading ${llmsContent.links.length} supplementary docs`)
          const supplementary = await downloadLlmsDocs(llmsContent, baseUrl, (_url, done, total) => {
            ctx.onProgress(`Downloading supplementary doc ${done + 1}/${total}`)
          })
          for (const doc of supplementary) {
            if (!ctx.isFrameworkDoc(doc.url))
              continue
            const localPath = doc.url.startsWith('/') ? doc.url.slice(1) : doc.url
            ctx.docs.push({ path: join('llms-docs', ...localPath.split('/')), content: doc.content })
          }
        }
      }
    }
  },
})
