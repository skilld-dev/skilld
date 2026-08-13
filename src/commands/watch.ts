import type { RepositoryRef } from '../registry/client.ts'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { loadSession } from '../auth/store.ts'
import { iterateSkills } from '../core/skills.ts'
import { createRegistryClient } from '../registry/client.ts'

type RepositoryInputResult
  = | { _tag: 'Ok', repositories: RepositoryRef[] }
    | { _tag: 'Err', invalid: string[] }

const REPOSITORY_RE = /^([\w.-]+)\/([\w.-]+)$/

export function parseRepositoryInputs(inputs: string[]): RepositoryInputResult {
  const repositories: RepositoryRef[] = []
  const invalid: string[] = []
  for (const input of inputs) {
    const match = input.trim().match(REPOSITORY_RE)
    if (!match) {
      invalid.push(input)
      continue
    }
    const repository = { owner: match[1]!, repo: match[2]! }
    if (!repositories.some(item => item.owner === repository.owner && item.repo === repository.repo))
      repositories.push(repository)
  }
  return invalid.length > 0 ? { _tag: 'Err', invalid } : { _tag: 'Ok', repositories }
}

export function installedRepositories(cwd = process.cwd()): RepositoryRef[] {
  const values = [...iterateSkills({ scope: 'local', cwd })]
    .flatMap(skill => skill.info?.repo ? [skill.info.repo] : [])
  return values.flatMap((value) => {
    const result = parseRepositoryInputs([value])
    return result._tag === 'Ok' ? result.repositories : []
  }).filter((repository, index, repositories) =>
    repositories.findIndex(candidate => candidate.owner === repository.owner && candidate.repo === repository.repo) === index,
  )
}

export type WatchRepositoriesResult
  = | { _tag: 'Watched', inserted: number }
    | { _tag: 'AuthRequired' }

export async function watchRepositories(repositories: RepositoryRef[]): Promise<WatchRepositoriesResult> {
  if (!await loadSession())
    return { _tag: 'AuthRequired' }
  const response = await createRegistryClient().my.watch(repositories)
  return { _tag: 'Watched', inserted: response.inserted }
}

function commandInputs(args: Record<string, unknown>): string[] {
  return [args.repository, ...((args._ as string[] | undefined) ?? [])]
    .filter((value): value is string => typeof value === 'string' && value.length > 0)
}

export const watchCommandDef = defineCommand({
  meta: { name: 'watch', description: 'Watch installed repositories for changes' },
  args: {
    repository: { type: 'positional', description: 'GitHub repository in owner/repo form', required: false },
  },
  async run({ args }) {
    const inputs = commandInputs(args)
    const parsed = inputs.length > 0
      ? parseRepositoryInputs(inputs)
      : { _tag: 'Ok' as const, repositories: installedRepositories() }
    if (parsed._tag === 'Err') {
      p.log.error(`Invalid repositories: ${parsed.invalid.join(', ')}. Use owner/repo.`)
      process.exitCode = 1
      return
    }
    if (parsed.repositories.length === 0) {
      p.log.warn('No installed GitHub repositories found.')
      return
    }
    const result = await watchRepositories(parsed.repositories).catch((error) => {
      p.log.error(`Failed to watch repositories: ${error instanceof Error ? error.message : String(error)}`)
      process.exitCode = 1
      return null
    })
    if (result?._tag === 'AuthRequired') {
      p.log.error('Not logged in. Run `skilld login` first.')
      process.exitCode = 1
    }
    if (result?._tag === 'Watched')
      p.log.success(`Watching ${parsed.repositories.length} repositories. ${result.inserted} added.`)
  },
})

export const unwatchCommandDef = defineCommand({
  meta: { name: 'unwatch', description: 'Stop watching a repository' },
  args: {
    repository: { type: 'positional', description: 'GitHub repository in owner/repo form', required: true },
  },
  async run({ args }) {
    const parsed = parseRepositoryInputs([args.repository])
    if (parsed._tag === 'Err') {
      p.log.error(`Invalid repository: ${args.repository}. Use owner/repo.`)
      process.exitCode = 1
      return
    }
    if (!await loadSession()) {
      p.log.error('Not logged in. Run `skilld login` first.')
      process.exitCode = 1
      return
    }
    await createRegistryClient().my.unwatch(parsed.repositories[0]!).then(() => {
      p.log.success(`Stopped watching ${args.repository}.`)
    }).catch((error) => {
      p.log.error(`Failed to stop watching: ${error instanceof Error ? error.message : String(error)}`)
      process.exitCode = 1
    })
  },
})
