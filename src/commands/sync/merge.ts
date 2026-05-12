/**
 * Merge mode — when a skill dir already holds a different primary package,
 * runBaseSync returns `merge-needed` and we regenerate SKILL.md combining
 * the existing primary with the new package. Sequential only.
 */

import type { AgentType } from '../../agent/index.ts'
import type { MergeNeededState } from './phases.ts'
import { existsSync } from 'node:fs'
import * as p from '@clack/prompts'
import { linkSkillToAgents, writeGeneratedSkillMd } from '../../agent/index.ts'
import { installSkill } from '../../agent/skill-installer.ts'
import { createReferenceCache } from '../../cache/index.ts'
import { getActiveFeatures } from '../../core/config.ts'
import { todayIsoDate } from '../../core/formatting.ts'
import { parsePackageNames, readLock } from '../../core/lockfile.ts'
import { getSharedSkillsDir, skillRefsSection } from '../../core/paths.ts'
import { toStoragePackageName } from '../../core/prefix.ts'
import { parseGitHubRepoSlug } from '../../core/url.ts'
import { findRelatedSkills } from './pipeline.ts'

export interface MergeConfig {
  agent: AgentType
  global: boolean
}

export async function handleMerge(
  state: MergeNeededState,
  config: MergeConfig,
  cwd: string,
): Promise<void> {
  const { identityName, storageName, version, resolved, baseDir, skillDir, skillDirName, existingLock } = state

  p.log.step(`Merging ${identityName} into ${skillDirName}`)
  const cache = createReferenceCache(storageName, version)
  cache.linkPkgNamed(skillDir, cwd)

  const repoSlug = parseGitHubRepoSlug(resolved.repoUrl)
  installSkill({
    cwd,
    agent: config.agent,
    global: config.global,
    baseDir,
    skillDirName,
    lock: {
      packageName: identityName,
      version,
      repo: repoSlug,
      source: existingLock.source,
      syncedAt: todayIsoDate(),
      generator: 'skilld',
    },
    skipLinkAgents: true,
  })

  const updatedLock = readLock(baseDir)?.skills[skillDirName]
  const allPackages = parsePackageNames(updatedLock?.packages)
  const relatedSkills = await findRelatedSkills(storageName, baseDir)
  const existingStorageName = toStoragePackageName(existingLock.packageName!)
  const existingCache = createReferenceCache(existingStorageName, existingLock.version)
  const pkgFiles = existingCache.keyFiles(cwd)
  const shippedDocs = existingCache.hasShipped(cwd)

  const features = getActiveFeatures()
  writeGeneratedSkillMd(skillDir, {
    name: existingLock.packageName!,
    version: existingLock.version,
    relatedSkills,
    hasIssues: features.issues && existsSync(skillRefsSection(skillDir, 'issues')),
    hasDiscussions: features.discussions && existsSync(skillRefsSection(skillDir, 'discussions')),
    hasReleases: features.releases && existsSync(skillRefsSection(skillDir, 'releases')),
    docsType: (existingLock.source?.includes('llms.txt') ? 'llms.txt' : 'docs') as 'llms.txt' | 'readme' | 'docs',
    hasShippedDocs: shippedDocs,
    pkgFiles,
    dirName: skillDirName,
    packages: allPackages,
    features,
  })

  const sharedDir = !config.global && getSharedSkillsDir(cwd)
  if (sharedDir)
    linkSkillToAgents(skillDirName, sharedDir, cwd, config.agent)

  p.outro(`Merged ${identityName} into ${skillDirName}`)
}
