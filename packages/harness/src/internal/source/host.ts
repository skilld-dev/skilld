import type { SkillOutputPolicy, SkillRunError, SourceAttempt } from '../../types.ts'
import type { Result } from '../result.ts'
import { constants } from 'node:fs'
import { lstat, open, opendir } from 'node:fs/promises'
import { join, relative, resolve, sep } from 'node:path'
import { err, ok } from '../result.ts'

export interface PreparedFile {
  readonly path: string
  readonly content: Uint8Array
}

export interface PreparedSource {
  readonly files: ReadonlyArray<PreparedFile>
  readonly attempts: ReadonlyArray<SourceAttempt>
  readonly npmResolution?: {
    readonly package: string
    readonly version: string
  }
}

const ignoredNames = new Set([
  '.git',
  '.hg',
  '.next',
  '.nuxt',
  '.output',
  '.skilld',
  '.turbo',
  'coverage',
  'dist',
  'node_modules',
  'target',
])

export async function collectHostDirectory(directory: string, policy: SkillOutputPolicy, sourceLabel = directory): Promise<Result<PreparedSource, SkillRunError>> {
  const root = resolve(directory)
  const unavailable = (message: string, cause?: unknown): Result<never, SkillRunError> => err({
    _tag: 'SourceUnavailable',
    message,
    attempts: [{ source: sourceLabel, status: 'skipped', reason: message }],
    cause,
  })
  const rootStat = await lstat(root).catch(error => error as NodeJS.ErrnoException)
  if (rootStat instanceof Error)
    return unavailable('Source directory is unavailable.', rootStat)
  if (!rootStat.isDirectory() || rootStat.isSymbolicLink())
    return unavailable('Source path must be a directory, not a symbolic link.')

  const files: PreparedFile[] = []
  let totalBytes = 0

  const walk = async (current: string): Promise<Result<void, SkillRunError>> => {
    const directoryHandle = await opendir(current).catch(error => error as NodeJS.ErrnoException)
    if (directoryHandle instanceof Error)
      return unavailable('Source directory cannot be read.', directoryHandle)

    for await (const entry of directoryHandle) {
      if (ignoredNames.has(entry.name))
        continue
      const absolute = join(current, entry.name)
      const stat = await lstat(absolute).catch(error => error as NodeJS.ErrnoException)
      if (stat instanceof Error)
        return unavailable('Source entry cannot be read.', stat)
      if (stat.isSymbolicLink())
        return unavailable('Source contains a symbolic link.')
      if (stat.isDirectory()) {
        const nested = await walk(absolute)
        if (nested._tag === 'Err')
          return nested
        continue
      }
      if (!stat.isFile())
        return unavailable('Source contains a special file.')

      const path = relative(root, absolute).split(sep).join('/')
      const handle = await open(absolute, constants.O_RDONLY | constants.O_NOFOLLOW).catch(error => error as NodeJS.ErrnoException)
      if (handle instanceof Error)
        return unavailable('Source file cannot be opened without following links.', handle)
      const openedStat = await handle.stat().catch(error => error as NodeJS.ErrnoException)
      if (openedStat instanceof Error) {
        await handle.close()
        return unavailable('Source file cannot be inspected.', openedStat)
      }
      if (!openedStat.isFile() || openedStat.dev !== stat.dev || openedStat.ino !== stat.ino) {
        await handle.close()
        return unavailable('Source file changed during collection.')
      }
      if (openedStat.size > policy.maxSourceFileBytes) {
        await handle.close()
        continue
      }
      if (files.length >= policy.maxSourceFiles) {
        await handle.close()
        return unavailable('Source contains too many files.')
      }
      if (totalBytes + openedStat.size > policy.maxSourceBytes) {
        await handle.close()
        return unavailable('Source exceeds the total byte limit.')
      }

      const content = await handle.readFile().catch(error => error as NodeJS.ErrnoException)
      await handle.close()
      if (content instanceof Error)
        return unavailable('Source file cannot be read.', content)
      if (content.byteLength !== openedStat.size)
        return unavailable('Source file changed during collection.')
      files.push({ path, content })
      totalBytes += content.byteLength
    }
    return ok(undefined)
  }

  const walked = await walk(root)
  if (walked._tag === 'Err')
    return walked
  if (files.length === 0)
    return unavailable('Source directory has no usable files.')

  files.sort((left, right) => left.path.localeCompare(right.path))
  return ok({ files, attempts: [{ source: sourceLabel, status: 'used' }] })
}
