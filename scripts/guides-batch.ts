/**
 * Batch-generate migration guides for the curated package set.
 *
 * Usage:
 *   tsx scripts/guides-batch.ts [--all] [--limit N] [--concurrency N]
 *                               [--out DIR] [--model M] [--force]
 *                               [--packages FILE]
 *
 * Writes <out>/<slug>.json (GeneratedGuide) + <out>/<slug>.md per package and a
 * <out>/_manifest.json summary. Re-runs skip already-generated guides unless
 * --force. Designed to run locally; skilld.dev ingests the JSON.
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'pathe'
import { listCuratedPackages } from '../src/guides/curated.ts'
import { generateGuide } from '../src/guides/generate.ts'
import { PIPELINE_VERSION } from '../src/guides/version.ts'

function arg(name: string, fallback?: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`)
  return i !== -1 && process.argv[i + 1] ? process.argv[i + 1] : fallback
}
function flag(name: string): boolean {
  return process.argv.includes(`--${name}`)
}

/** Filesystem-safe slug: `@scope/name` → `@scope__name`. */
function safeSlug(pkg: string): string {
  return pkg.replace(/\//g, '__')
}

const all = flag('all')
const force = flag('force')
const limit = Number(arg('limit') ?? Infinity)
const out = arg('out') ?? '.guides-out'
const model = (arg('model') ?? 'sonnet') as any
const distillModel = arg('distill-model') as any
const packagesFile = arg('packages')

// Local Ollama serializes requests, so parallel workers give no speedup and can
// thrash VRAM — default to 1 for `ollama:` models, 3 for cloud APIs.
const isLocal = String(model).startsWith('ollama:')
const concurrency = Number(arg('concurrency') ?? (isLocal ? '1' : '3'))
// Local models (esp. prose repos with many flagged LLM calls) are slow; give
// them plenty of headroom per guide.
const timeout = Number(arg('timeout') ?? (isLocal ? '1200000' : '300000'))

const packages = (packagesFile
  ? readFileSync(packagesFile, 'utf8').split('\n').map(l => l.trim()).filter(l => l && !l.startsWith('#'))
  : listCuratedPackages({ all })
).slice(0, limit)

mkdirSync(out, { recursive: true })

interface Row { pkg: string, status: 'ok' | 'skip' | 'fail', version?: string, cost?: number, error?: string }
const rows: Row[] = []
let totalCost = 0
let cursor = 0

async function worker(id: number): Promise<void> {
  while (cursor < packages.length) {
    const pkg = packages[cursor++]!
    const jsonPath = join(out, `${safeSlug(pkg)}.json`)

    // Incremental: skip a guide only when its persisted pipeline stamp matches
    // the current logic version. A version bump (bucketing/prompt) marks older
    // guides stale → they regenerate; everything else is skipped. --force redoes all.
    if (!force && existsSync(jsonPath)) {
      const stamp = JSON.parse(readFileSync(jsonPath, 'utf8'))?.pipelineVersion
      if (stamp === PIPELINE_VERSION) {
        rows.push({ pkg, status: 'skip' })
        process.stderr.write(`  [${id}] ⤏ skip   ${pkg} (current: ${stamp})\n`)
        continue
      }
      process.stderr.write(`  [${id}] ↻ stale  ${pkg} (${stamp ?? 'unstamped'} → ${PIPELINE_VERSION})\n`)
    }

    process.stderr.write(`  [${id}] … gen    ${pkg}\n`)
    const result = await generateGuide(pkg, { model, distillModel, timeout }).catch(err => ({ ok: false as const, error: String(err?.message ?? err) }))

    if (!result.ok) {
      rows.push({ pkg, status: 'fail', error: result.error })
      process.stderr.write(`  [${id}] ✗ fail   ${pkg}: ${result.error}\n`)
      continue
    }

    const { guide } = result
    writeFileSync(jsonPath, JSON.stringify(guide, null, 2))
    writeFileSync(join(out, `${safeSlug(pkg)}.md`), guide.markdown)
    totalCost += guide.cost ?? 0
    const c = guide.counts
    rows.push({ pkg, status: 'ok', version: guide.version, cost: guide.cost })
    process.stderr.write(`  [${id}] ✓ ok     ${pkg}@${guide.version} [${c.breaking}b/${c.features}f/${c.fixes}x/${c.improvements}i]${guide.cost ? ` ($${guide.cost.toFixed(3)})` : ''}\n`)
  }
}

process.stderr.write(`Generating ${packages.length} guides (model ${model}${distillModel ? `, distill ${distillModel}` : ''}, concurrency ${concurrency}) → ${out}\n\n`)
await Promise.all(Array.from({ length: Math.min(concurrency, packages.length) }, (_, i) => worker(i + 1)))

const ok = rows.filter(r => r.status === 'ok').length
const skip = rows.filter(r => r.status === 'skip').length
const fail = rows.filter(r => r.status === 'fail')
writeFileSync(join(out, '_manifest.json'), JSON.stringify({ generatedCount: ok, skipped: skip, failed: fail.length, totalCost, rows }, null, 2))

process.stderr.write(`\n── done ──\n  ok=${ok} skip=${skip} fail=${fail.length} cost=$${totalCost.toFixed(2)}\n`)
if (fail.length)
  process.stderr.write(`  failures:\n${fail.map(f => `    - ${f.pkg}: ${f.error}`).join('\n')}\n`)
