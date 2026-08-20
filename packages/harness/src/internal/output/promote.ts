import type { GeneratedSkill, SkillRunError, SourceAttempt } from '../../types.ts'
import type { Result } from '../result.ts'
import type { CollectedFile } from './collect.ts'
import { randomUUID } from 'node:crypto'
import { lstat, mkdir, open, realpath, rename, rm, rmdir, unlink, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { err, ok } from '../result.ts'

function pathError(message: string, path: string): Result<never, SkillRunError> {
  return err({ _tag: 'UnsafeOutputPath', message, path })
}

const missing = Symbol('missing')

async function statOrMissing(path: string) {
  return lstat(path).catch(error => (error as NodeJS.ErrnoException).code === 'ENOENT' ? missing : Promise.reject(error))
}

export const promoteSkill = async (
  rootDir: string,
  name: string,
  files: ReadonlyArray<CollectedFile>,
  attempts: ReadonlyArray<SourceAttempt>,
): Promise<Result<GeneratedSkill, SkillRunError>> => {
  const root = resolve(rootDir)
  let rootStat = await statOrMissing(root).catch(error => error as Error)
  if (rootStat instanceof Error)
    return pathError('Output root cannot be inspected.', root)
  let createdRoot = false
  if (rootStat === missing) {
    const parent = dirname(root)
    const parentStat = await statOrMissing(parent).catch(error => error as Error)
    if (parentStat instanceof Error || parentStat === missing || !parentStat.isDirectory() || parentStat.isSymbolicLink())
      return pathError('Output root parent must be an existing directory.', parent)
    const canonicalParent = await realpath(parent).catch(error => error as Error)
    if (canonicalParent instanceof Error || canonicalParent !== parent)
      return pathError('Output root parent must not pass through a symbolic link.', parent)
    const created = await mkdir(root, { mode: 0o700 }).then(() => true, error => error as NodeJS.ErrnoException)
    if (created instanceof Error && created.code !== 'EEXIST')
      return pathError('Output root cannot be created.', root)
    createdRoot = created === true
    rootStat = await statOrMissing(root).catch(error => error as Error)
  }
  if (rootStat instanceof Error || rootStat === missing || !rootStat.isDirectory() || rootStat.isSymbolicLink())
    return pathError('Output root must be a directory, not a symbolic link.', root)
  const canonicalRoot = await realpath(root).catch(error => error as Error)
  if (canonicalRoot instanceof Error)
    return pathError('Output root cannot be resolved.', root)
  if (canonicalRoot !== root)
    return pathError('Output root must not pass through a symbolic link.', root)

  const target = join(root, name)
  const targetStat = await statOrMissing(target).catch(error => error as Error)
  if (targetStat instanceof Error)
    return pathError('Output path cannot be inspected.', target)
  if (targetStat !== missing && (!targetStat.isDirectory() || targetStat.isSymbolicLink()))
    return pathError('Output path must be a directory, not a symbolic link.', target)

  const lockPath = join(root, `.skilld-${name}.lock`)
  const lock = await open(lockPath, 'wx').catch(error => error as NodeJS.ErrnoException)
  if (lock instanceof Error) {
    if (lock.code === 'EEXIST')
      return err({ _tag: 'OutputBusy', message: 'Another Skill run owns this output path.', path: target })
    return err({ _tag: 'PromotionFailed', message: 'Output lock could not be created.', path: target, cause: lock })
  }

  const nonce = randomUUID()
  const staging = join(root, `.skilld-${name}-${nonce}.next`)
  const backup = join(root, `.skilld-${name}-${nonce}.previous`)
  let movedCurrent = false
  let promoted = false
  let failureMessage = 'Skill output could not be promoted.'
  let failureCause: unknown

  try {
    await mkdir(staging, { mode: 0o700 })
    for (const file of files) {
      const destination = join(staging, file.path)
      await mkdir(dirname(destination), { recursive: true, mode: 0o700 })
      await writeFile(destination, file.content, { flag: 'wx', mode: 0o600 })
    }

    if (targetStat !== missing) {
      await rename(target, backup)
      movedCurrent = true
    }
    await rename(staging, target)
    promoted = true
  }
  catch (cause) {
    failureCause = cause
    if (movedCurrent && !promoted) {
      const current = await statOrMissing(target).catch(error => error as Error)
      if (current instanceof Error) {
        failureMessage = 'Skill output rollback could not inspect its destination.'
        failureCause = new AggregateError([cause, current])
      }
      if (current === missing) {
        const rollback = await rename(backup, target).catch(error => error as Error)
        if (rollback instanceof Error) {
          failureMessage = 'Skill output rollback failed.'
          failureCause = new AggregateError([cause, rollback])
        }
      }
    }
  }

  const lockCleanupErrors: unknown[] = []
  await lock.close().catch(error => lockCleanupErrors.push(error))
  await unlink(lockPath).catch(error => lockCleanupErrors.push(error))

  if (promoted) {
    const warnings: string[] = []
    if (lockCleanupErrors.length > 0) {
      const detail = lockCleanupErrors.map(error => error instanceof Error ? error.message : String(error)).join('; ')
      warnings.push(`Output lock cleanup failed at ${lockPath}: ${detail}`)
    }
    if (movedCurrent) {
      const backupCleanup = await rm(backup, { recursive: true }).catch(error => error as Error)
      if (backupCleanup instanceof Error)
        warnings.push(`Previous Skill cleanup failed at ${backup}: ${backupCleanup.message}`)
    }
    return ok({
      _tag: 'GeneratedSkill',
      name,
      outputDir: target,
      files: files.map(file => ({ path: file.path, bytes: file.content.byteLength })),
      sourceAttempts: attempts,
      warnings,
    })
  }

  const cleanupErrors = [...lockCleanupErrors]
  await rm(staging, { recursive: true, force: true }).catch(error => cleanupErrors.push(error))
  if (createdRoot)
    await rmdir(root).catch(error => cleanupErrors.push(error))
  const causes = failureCause === undefined ? cleanupErrors : [failureCause, ...cleanupErrors]
  const cause = causes.length <= 1 ? causes[0] : new AggregateError(causes, 'Skill output cleanup failed.')
  return err({ _tag: 'PromotionFailed', message: failureMessage, path: target, cause })
}
