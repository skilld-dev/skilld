import type { SkillOutputPolicy, SkillRunError, SourceAttempt } from '../../types.ts'
import type { Result } from '../result.ts'
import { constants } from 'node:fs'
import { lstat, open, opendir, realpath } from 'node:fs/promises'
import { join, relative, resolve, sep } from 'node:path'
import { err, ok } from '../result.ts'

export interface PreparedFile {
  readonly path: string
  readonly content: Uint8Array
}

/** A source file the Harness left out because it exceeds the file byte limit. */
export interface SkippedSourceFile {
  readonly path: string
  readonly bytes: number
}

export interface PreparedSource {
  readonly files: ReadonlyArray<PreparedFile>
  /** Files left out for size. The Agent and the caller both see this list. */
  readonly skippedFiles: ReadonlyArray<SkippedSourceFile>
  readonly attempts: ReadonlyArray<SourceAttempt>
  readonly npmResolution?: {
    readonly package: string
    readonly version: string
  }
}

/** One warning per skipped file, in the words the result reports. */
export function skippedFileWarnings(skipped: ReadonlyArray<SkippedSourceFile>, policy: SkillOutputPolicy): ReadonlyArray<string> {
  return skipped.map(file =>
    `Source file ${file.path} was left out: ${file.bytes} bytes exceeds the ${policy.maxSourceFileBytes} byte file limit.`)
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

export async function collectHostDirectory(directory: string, policy: SkillOutputPolicy, sourceLabel = directory, signal?: AbortSignal): Promise<Result<PreparedSource, SkillRunError>> {
  const root = resolve(directory)
  const unavailable = (message: string, cause?: unknown): Result<never, SkillRunError> => err({
    _tag: 'SourceUnavailable',
    message,
    attempts: [{ source: sourceLabel, status: 'skipped', reason: message }],
    cause,
  })
  const cancelled = (): Result<never, SkillRunError> => err({ _tag: 'Cancelled', message: 'Skill run was cancelled.' })
  if (signal?.aborted)
    return cancelled()
  const rootStat = await lstat(root).catch(error => error as NodeJS.ErrnoException)
  if (rootStat instanceof Error)
    return unavailable('Source directory is unavailable.', rootStat)
  if (!rootStat.isDirectory() || rootStat.isSymbolicLink())
    return unavailable('Source path must be a directory, not a symbolic link.')
  const canonicalRoot = await realpath(root).catch(error => error as NodeJS.ErrnoException)
  if (canonicalRoot instanceof Error)
    return unavailable('Source directory cannot be resolved.', canonicalRoot)
  if (canonicalRoot !== root)
    return unavailable('Source path must not pass through a symbolic link.')

  const files: PreparedFile[] = []
  const skippedFiles: SkippedSourceFile[] = []
  let totalBytes = 0
  const relativePath = (absolute: string): string => relative(root, absolute).split(sep).join('/')
  /** An entry-level failure names the entry, so the caller can find and fix it. */
  const entryUnavailable = (absolute: string, message: string, cause?: unknown, fix?: string): Result<never, SkillRunError> =>
    unavailable(`${message}: ${relativePath(absolute)}.${fix === undefined ? '' : ` ${fix}`}`, cause)

  const walk = async (current: string): Promise<Result<void, SkillRunError>> => {
    if (signal?.aborted)
      return cancelled()
    const canonicalCurrent = await realpath(current).catch(error => error as NodeJS.ErrnoException)
    if (canonicalCurrent instanceof Error || canonicalCurrent !== current)
      return unavailable('Source directory changed during collection.', canonicalCurrent instanceof Error ? canonicalCurrent : undefined)
    const directoryHandle = await opendir(current).catch(error => error as NodeJS.ErrnoException)
    if (directoryHandle instanceof Error)
      return unavailable('Source directory cannot be read.', directoryHandle)

    for await (const entry of directoryHandle) {
      if (signal?.aborted)
        return cancelled()
      if (ignoredNames.has(entry.name))
        continue
      const absolute = join(current, entry.name)
      const stat = await lstat(absolute).catch(error => error as NodeJS.ErrnoException)
      if (stat instanceof Error)
        return entryUnavailable(absolute, 'Source entry cannot be read', stat)
      if (stat.isSymbolicLink())
        return entryUnavailable(absolute, 'Source contains a symbolic link', undefined, 'Remove the link, or pass a clean export such as a `git archive` of the directory.')
      if (stat.isDirectory()) {
        const nested = await walk(absolute)
        if (nested._tag === 'Err')
          return nested
        continue
      }
      if (!stat.isFile())
        return entryUnavailable(absolute, 'Source contains a special file')

      const path = relativePath(absolute)
      const handle = await open(absolute, constants.O_RDONLY | constants.O_NOFOLLOW).catch(error => error as NodeJS.ErrnoException)
      if (handle instanceof Error)
        return entryUnavailable(absolute, 'Source file cannot be opened without following links', handle)
      const openedStat = await handle.stat().catch(error => error as NodeJS.ErrnoException)
      if (openedStat instanceof Error) {
        await handle.close()
        return entryUnavailable(absolute, 'Source file cannot be inspected', openedStat)
      }
      if (!openedStat.isFile() || openedStat.dev !== stat.dev || openedStat.ino !== stat.ino) {
        await handle.close()
        return entryUnavailable(absolute, 'Source file changed during collection')
      }
      if (openedStat.size > policy.maxSourceFileBytes) {
        await handle.close()
        skippedFiles.push({ path, bytes: openedStat.size })
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
        return entryUnavailable(absolute, 'Source file cannot be read', content)
      if (content.byteLength !== openedStat.size)
        return entryUnavailable(absolute, 'Source file changed during collection')
      if (signal?.aborted)
        return cancelled()
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
  skippedFiles.sort((left, right) => left.path.localeCompare(right.path))
  return ok({ files, skippedFiles, attempts: [{ source: sourceLabel, status: 'used' }] })
}
