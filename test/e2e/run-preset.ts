/**
 * Shared test generator for preset-based e2e tests.
 *
 * Each preset file calls `describePreset('name')` to run the full
 * sync pipeline against all packages in that preset group.
 */

import type { Preset } from './matrix'
import type { PipelineResult } from './pipeline'
import { existsSync, mkdirSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'pathe'
import { beforeAll, describe, expect, it } from 'vitest'
import { computeSkillDirName } from '../../src/agent'
import {
  ensureCacheDir,
  getCacheDir,
  getPackageDbPath,
} from '../../src/cache'
import { getShippedSkills } from '../../src/core/prepare'
import { search } from '../../src/retriv'
import { PACKAGES } from './matrix'
import { hasValidSearchDb, parseFrontmatter, runPipeline } from './pipeline'

const ARTIFACTS_DIR = resolve(import.meta.dirname, '../../.artifacts')

function writeArtifact(preset: string, name: string, result: PipelineResult) {
  const safeName = name.replace(/\//g, '__')
  const dir = join(ARTIFACTS_DIR, preset, safeName)
  mkdirSync(dir, { recursive: true })

  writeFileSync(join(dir, 'result.json'), JSON.stringify({
    name: result.resolved.name,
    version: result.version,
    docsType: result.docsType,
    repoUrl: result.resolved.repoUrl,
    docsUrl: result.resolved.docsUrl,
    llmsUrl: result.resolved.llmsUrl,
    gitDocsUrl: result.resolved.gitDocsUrl,
    gitRef: result.resolved.gitRef,
    readmeUrl: result.resolved.readmeUrl,
    cachedDocsCount: result.cachedDocsCount,
    attempts: result.attempts,
  }, null, 2))

  writeFileSync(join(dir, 'cached-files.txt'), result.cachedFiles.join('\n'))
  writeFileSync(join(dir, 'SKILL.md'), result.skillMd)
}

function writeError(preset: string, name: string, error: Error) {
  const safeName = name.replace(/\//g, '__')
  const dir = join(ARTIFACTS_DIR, preset, safeName)
  mkdirSync(dir, { recursive: true })
  writeFileSync(join(dir, 'error.txt'), error.message)
}

export function describePreset(presetName: Preset) {
  const packages = PACKAGES.filter(p => p.preset === presetName)

  describe(`e2e ${presetName} preset`, () => {
    const results = new Map<string, PipelineResult>()
    const errors = new Map<string, Error>()

    beforeAll(async () => {
      ensureCacheDir()
      await Promise.allSettled(
        packages.map(async (pkg) => {
          try {
            const result = await runPipeline(pkg.name)
            results.set(pkg.name, result)
            writeArtifact(presetName, pkg.name, result)
          }
          catch (err) {
            errors.set(pkg.name, err as Error)
            writeError(presetName, pkg.name, err as Error)
          }
        }),
      )
    }, 180_000)

    for (const pkg of packages) {
      describe(pkg.name, () => {
        function get(): PipelineResult {
          const err = errors.get(pkg.name)
          if (err)
            throw err
          const r = results.get(pkg.name)
          if (!r)
            throw new Error(`No result for ${pkg.name}`)
          return r
        }

        // ── Resolution ──

        it('resolves on npm with version', () => {
          const r = get()
          expect(r.resolved.name).toBe(pkg.name)
          expect(r.resolved.version).toMatch(/^\d+\.\d+/)
        })

        it(`repo → ${pkg.expectRepoUrl}`, () => {
          expect(get().resolved.repoUrl).toContain(pkg.expectRepoUrl)
        })

        if (pkg.expectDocsUrl) {
          it(`docsUrl → ${pkg.expectDocsUrl}`, () => {
            expect(get().resolved.docsUrl).toBe(pkg.expectDocsUrl)
          })
        }
        else {
          it('no docsUrl', () => {
            expect(get().resolved.docsUrl).toBeFalsy()
          })
        }

        it('resolution sources', () => {
          const r = get()
          const { expectSources } = pkg

          if (expectSources.gitDocs) {
            expect(r.resolved.gitDocsUrl).toBeTruthy()
            expect(r.resolved.gitRef).toBeTruthy()
          }
          else {
            expect(r.resolved.gitDocsUrl).toBeFalsy()
          }

          if (expectSources.llmsTxt) {
            expect(r.resolved.llmsUrl).toBeTruthy()
          }
          else {
            expect(r.resolved.llmsUrl).toBeFalsy()
          }

          if (expectSources.readme) {
            expect(r.resolved.readmeUrl).toBeTruthy()
          }
        })

        // ── Shipped skills ──

        if (pkg.expectShipped) {
          it('getShippedSkills() returns skills', () => {
            const cwd = process.cwd()
            const shipped = getShippedSkills(pkg.name, cwd)
            expect(shipped.length).toBeGreaterThan(0)
            for (const name of pkg.expectShippedSkills || []) {
              expect(shipped.some(s => s.skillName === name)).toBe(true)
            }
          })

          for (const skillName of pkg.expectShippedSkills || []) {
            it(`shipped skill "${skillName}" has SKILL.md`, () => {
              const cwd = process.cwd()
              const shipped = getShippedSkills(pkg.name, cwd)
              const match = shipped.find(s => s.skillName === skillName)!
              expect(existsSync(join(match.skillDir, 'SKILL.md'))).toBe(true)
            })

            it(`shipped skill "${skillName}" has .skilld/`, () => {
              const cwd = process.cwd()
              const shipped = getShippedSkills(pkg.name, cwd)
              const match = shipped.find(s => s.skillName === skillName)!
              expect(existsSync(join(match.skillDir, '.skilld'))).toBe(true)
            })
          }
        }

        // ── Cache (skip for shipped packages) ──

        if (!pkg.expectShipped) {
          it(`docs type → ${pkg.expectDocsType}`, () => {
            expect(get().docsType).toBe(pkg.expectDocsType)
          })

          it(`≥${pkg.minCacheDocs} cached docs`, () => {
            expect(get().cachedDocsCount).toBeGreaterThanOrEqual(pkg.minCacheDocs)
          })

          it('cache dir exists', () => {
            const r = get()
            expect(existsSync(getCacheDir(pkg.name, r.version))).toBe(true)
          })

          for (const file of pkg.expectCacheFiles) {
            it(`cached: ${file}`, () => {
              const r = get()
              const cacheDir = getCacheDir(pkg.name, r.version)
              expect(existsSync(join(cacheDir, file))).toBe(true)
            })
          }
        }

        // ── SKILL.md (skip for shipped packages) ──

        if (!pkg.expectShipped) {
          it('valid frontmatter', () => {
            const r = get()
            const fm = parseFrontmatter(r.skillMd)
            const expectedDirName = computeSkillDirName(pkg.name)
            expect(fm.name).toBe(expectedDirName)
            expect(fm.description).toBeTruthy()
          })

          it(`description contains ${pkg.expectDescriptionContains}`, () => {
            const fm = parseFrontmatter(get().skillMd)
            expect(fm.description).toContain(pkg.expectDescriptionContains)
          })
        }

        // ── Search index (skip for shipped packages, skip in CI — ONNX model unreliable) ──

        if (!pkg.expectShipped && !process.env.CI) {
          it('search.db valid if present', () => {
            const r = get()
            if (r.docsType === 'llms.txt')
              return
            const dbPath = getPackageDbPath(pkg.name, r.version)
            if (!hasValidSearchDb(dbPath))
              return // No valid index — skip
            expect(hasValidSearchDb(dbPath)).toBe(true)
          })

          if (pkg.searchQuery) {
            it(`search("${pkg.searchQuery.query}") ≥${pkg.searchQuery.minHits} hits`, async () => {
              const r = get()
              const dbPath = getPackageDbPath(pkg.name, r.version)
              if (!hasValidSearchDb(dbPath))
                return
              const hits = await search(pkg.searchQuery!.query, { dbPath }, { limit: 5 })
              expect(hits.length).toBeGreaterThanOrEqual(pkg.searchQuery!.minHits)
            })
          }
        }
      })
    }
  })
}
