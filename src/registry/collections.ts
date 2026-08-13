import type { SkillSource } from '../core/prefix.ts'
import type { CollectionManifestItem } from './client.ts'

/**
 * Convert selected manifest items into `installSkills` inputs. Multiple gh
 * items in the same repo collapse to one `git` source carrying the union of
 * picked skill names as `skillFilter`, so a repo with N skills installs in
 * one `syncGitSkills` call instead of N redundant ones.
 */
export function manifestToSources(items: CollectionManifestItem[]): Array<{ source: SkillSource, skillFilter?: string }> {
  const npm: Array<{ source: SkillSource, skillFilter?: string }> = []
  const crate: Array<{ source: SkillSource, skillFilter?: string }> = []
  const ghByRepo = new Map<string, { owner: string, repo: string, names: string[] }>()

  for (const item of items) {
    if (item.kind === 'npm' && item.package) {
      npm.push({ source: { type: 'npm', package: item.package } })
      continue
    }
    if (item.kind === 'crate' && item.package) {
      crate.push({ source: { type: 'crate', package: item.package } })
      continue
    }
    if (item.kind === 'gh' && item.owner && item.repo) {
      const key = `${item.owner}/${item.repo}`
      const group = ghByRepo.get(key) ?? { owner: item.owner, repo: item.repo, names: [] }
      if (item.name && !group.names.includes(item.name))
        group.names.push(item.name)
      ghByRepo.set(key, group)
    }
  }

  const gh: Array<{ source: SkillSource, skillFilter?: string }> = []
  for (const group of ghByRepo.values()) {
    gh.push({
      source: { type: 'git', source: { type: 'github', owner: group.owner, repo: group.repo } },
      skillFilter: group.names.length ? group.names.join(',') : undefined,
    })
  }

  return [...gh, ...npm, ...crate]
}
