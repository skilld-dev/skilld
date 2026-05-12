/**
 * Local readme fallback resolver — last-ditch lookup in `node_modules`.
 *
 * Runs only when nothing else has populated docs/llms/readme/gitDocs and a
 * `cwd` is available. Finds `README.md` under `node_modules/<pkg>/` and
 * records it as a `file://` URL.
 */

import { existsSync, readdirSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import { join } from 'pathe'
import { defineResolver } from '../resolver-registry.ts'

export const localReadmeResolver = defineResolver({
  id: 'local',
  canResolve: (ctx) => {
    const r = ctx.result
    return !!r && !!ctx.options.cwd && !r.docsUrl && !r.llmsUrl && !r.readmeUrl && !r.gitDocsUrl
  },
  async run(ctx) {
    const result = ctx.result!
    ctx.options.onProgress?.('local')
    const pkgDir = join(ctx.options.cwd!, 'node_modules', ctx.packageName)
    const readmeFile = existsSync(pkgDir) && readdirSync(pkgDir).find(f => /^readme\.md$/i.test(f))
    if (readmeFile) {
      const readmePath = join(pkgDir, readmeFile)
      result.readmeUrl = pathToFileURL(readmePath).href
      ctx.attempts.push({
        source: 'readme',
        url: readmePath,
        status: 'success',
        message: 'Found local readme in node_modules',
      })
      return { kind: 'ok' }
    }
    return { kind: 'skip' }
  },
})
