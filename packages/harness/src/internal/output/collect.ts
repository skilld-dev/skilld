import type { SkillOutputPolicy, SkillRunError } from '../../types.ts'
import type { Result } from '../result.ts'
import type { HarnessAgentSandboxConfig } from '@ai-sdk/harness/agent'
import { normalizeOutputPath } from '../paths.ts'
import { err, ok } from '../result.ts'

export type SandboxSession = Parameters<NonNullable<HarnessAgentSandboxConfig['onSession']>>[0]['session']

export interface CollectedFile {
  readonly path: string
  readonly content: Uint8Array
}

const invalid = (message: string, issues: ReadonlyArray<string>): Result<never, SkillRunError> =>
  err({ _tag: 'InvalidSkill', message, issues })

const inventory = async (
  sandbox: SandboxSession,
  outputDir: string,
  signal?: AbortSignal,
): Promise<Result<ReadonlyArray<{ type: string, path: string, size: number }>, SkillRunError>> => {
  const command = 'find -P "$SKILLD_OUTPUT" -mindepth 1 -printf \'%y\\0%P\\0%s\\0\''
  const result = await Promise.resolve(sandbox.run({
    command,
    env: { LC_ALL: 'C', SKILLD_OUTPUT: outputDir },
    abortSignal: signal,
  })).catch(error => error as Error)
  if (result instanceof Error)
    return invalid('Harness output cannot be inspected.', [result.message])
  if (result.exitCode !== 0)
    return invalid('Harness output directory is missing.', [result.stderr.trim() || 'No output directory.'])

  const fields = result.stdout.split('\0')
  if (fields.at(-1) === '')
    fields.pop()
  if (fields.length % 3 !== 0)
    return invalid('Harness output inventory is invalid.', ['File inventory has an incomplete entry.'])

  const entries: Array<{ type: string, path: string, size: number }> = []
  for (let index = 0; index < fields.length; index += 3) {
    const type = fields[index]
    const path = fields[index + 1]
    const size = Number(fields[index + 2])
    if (!type || !path || !Number.isSafeInteger(size) || size < 0)
      return invalid('Harness output inventory is invalid.', ['File inventory has invalid fields.'])
    entries.push({ type, path, size })
  }
  return ok(entries)
}

export const collectSandboxOutput = async (
  sandbox: SandboxSession,
  outputDir: string,
  policy: SkillOutputPolicy,
  signal?: AbortSignal,
): Promise<Result<ReadonlyArray<CollectedFile>, SkillRunError>> => {
  const before = await inventory(sandbox, outputDir, signal)
  if (before._tag === 'Err')
    return before

  const files: CollectedFile[] = []
  const seen = new Set<string>()
  let totalBytes = 0
  for (const entry of before.value) {
    const path = normalizeOutputPath(entry.path)
    if (path === null)
      return invalid('Harness output contains an invalid path.', [entry.path])
    if (entry.type === 'd')
      continue
    if (entry.type !== 'f')
      return invalid('Harness output contains a linked or special entry.', [path])
    if (seen.has(path))
      return invalid('Harness output contains duplicate paths.', [path])
    if (files.length >= policy.maxOutputFiles)
      return invalid('Harness output contains too many files.', [`Limit: ${policy.maxOutputFiles}`])
    if (entry.size > policy.maxOutputFileBytes)
      return invalid('Harness output contains an oversized file.', [path])
    if (totalBytes + entry.size > policy.maxOutputBytes)
      return invalid('Harness output exceeds the total byte limit.', [`Limit: ${policy.maxOutputBytes}`])

    const content = await sandbox.readBinaryFile({ path: `${outputDir}/${path}`, abortSignal: signal })
    if (content === null || content.byteLength !== entry.size)
      return invalid('Harness output changed during collection.', [path])
    seen.add(path)
    files.push({ path, content: Uint8Array.from(content) })
    totalBytes += content.byteLength
  }

  const after = await inventory(sandbox, outputDir, signal)
  if (after._tag === 'Err')
    return after
  if (JSON.stringify(before.value) !== JSON.stringify(after.value))
    return invalid('Harness output changed during collection.', ['File inventory changed.'])

  files.sort((left, right) => left.path.localeCompare(right.path))
  return ok(files)
}
