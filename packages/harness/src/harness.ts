import type { HarnessV1Skill } from '@ai-sdk/harness'
import type { SandboxSession } from './internal/output/collect.ts'
import type { Result } from './internal/result.ts'
import type { PreparedSource } from './internal/source/host.ts'
import type { FetchClient } from './internal/source/npm.ts'
import type {
  CreateSkillHarnessOptions,
  SkillDestination,
  SkillHarness,
  SkillOutputPolicy,
  SkillRun,
  SkillRunError,
  SkillRunResult,
} from './types.ts'
import { lstat, realpath } from 'node:fs/promises'
import { join, posix, resolve } from 'node:path'
import { HarnessAgent } from '@ai-sdk/harness/agent'
import { parseOutputPolicy, parseSkillRun } from './internal/input.ts'
import { collectSandboxOutput } from './internal/output/collect.ts'
import { promoteSkill } from './internal/output/promote.ts'
import { validateGeneratedSkill, validateSkillReview } from './internal/output/validate.ts'
import { resolveWithin } from './internal/paths.ts'
import { err, ok } from './internal/result.ts'
import { collectHostDirectory } from './internal/source/host.ts'
import { prepareNpmPackage } from './internal/source/npm.ts'
import { DEFAULT_OUTPUT_POLICY, loadSkilldMaintainedSkill } from './skills.ts'

interface PreparedRun {
  readonly skillName: 'generate-package-skill' | 'generate-project-skill' | 'review-skill'
  readonly outputName: string
  readonly source: PreparedSource
  readonly current?: PreparedSource
  readonly destination?: SkillDestination
}

interface ActiveSandbox {
  readonly sandbox: SandboxSession
  readonly workDir: string
}

function cancelled(): SkillRunResult {
  return err({ _tag: 'Cancelled', message: 'Skill run was cancelled.' })
}

async function prepareCurrentSkill(destination: SkillDestination, policy: SkillOutputPolicy, signal?: AbortSignal): Promise<Result<PreparedSource | undefined, SkillRunError>> {
  const root = resolve(destination.rootDir)
  const rootStat = await lstat(root).catch(error => error as NodeJS.ErrnoException)
  if (rootStat instanceof Error) {
    if (rootStat.code === 'ENOENT')
      return ok(undefined)
    return err({ _tag: 'UnsafeOutputPath', message: 'Output root cannot be inspected.', path: root })
  }
  if (!rootStat.isDirectory() || rootStat.isSymbolicLink())
    return err({ _tag: 'UnsafeOutputPath', message: 'Output root must be a directory, not a symbolic link.', path: root })
  const canonicalRoot = await realpath(root).catch(error => error as Error)
  if (canonicalRoot instanceof Error || canonicalRoot !== root)
    return err({ _tag: 'UnsafeOutputPath', message: 'Output root must not pass through a symbolic link.', path: root })

  const target = join(root, destination.name)
  const targetStat = await lstat(target).catch(error => error as NodeJS.ErrnoException)
  if (targetStat instanceof Error) {
    if (targetStat.code === 'ENOENT')
      return ok(undefined)
    return err({ _tag: 'UnsafeOutputPath', message: 'Output path cannot be inspected.', path: target })
  }
  if (!targetStat.isDirectory() || targetStat.isSymbolicLink())
    return err({ _tag: 'UnsafeOutputPath', message: 'Output path must be a directory, not a symbolic link.', path: target })
  return collectHostDirectory(target, policy, 'current Skill', signal)
}

async function prepareRun(input: SkillRun, policy: SkillOutputPolicy, fetchClient: FetchClient, signal?: AbortSignal): Promise<Result<PreparedRun, SkillRunError>> {
  if (signal?.aborted)
    return err({ _tag: 'Cancelled', message: 'Skill run was cancelled.' })

  const current = input._tag === 'ReviewSkill'
    ? ok(undefined)
    : await prepareCurrentSkill(input.destination, policy, signal)
  if (current._tag === 'Err')
    return current

  if (input._tag === 'ProjectSkill') {
    const source = await collectHostDirectory(input.projectDir, policy, input.projectDir, signal)
    return source._tag === 'Err'
      ? source
      : ok({ skillName: 'generate-project-skill', outputName: input.destination.name, source: source.value, current: current.value, destination: input.destination })
  }

  if (input._tag === 'ReviewSkill') {
    const source = await collectHostDirectory(input.skillDir, policy, input.skillDir, signal)
    return source._tag === 'Err'
      ? source
      : ok({ skillName: 'review-skill', outputName: 'review', source: source.value })
  }

  if (input.source._tag === 'NpmPackage') {
    const source = await prepareNpmPackage(input.source.spec, policy, fetchClient, signal)
    if (signal?.aborted)
      return err({ _tag: 'Cancelled', message: 'Skill run was cancelled.' })
    return source._tag === 'Err'
      ? source
      : ok({ skillName: 'generate-package-skill', outputName: input.destination.name, source: source.value, current: current.value, destination: input.destination })
  }

  const packageDir = resolveWithin(input.source.rootDir, input.source.packageDir)
  if (packageDir === null)
    return err({ _tag: 'InvalidInput', message: 'Local package directory must stay inside its root directory.' })
  const manifest = await lstat(join(packageDir, 'package.json')).catch(error => error as NodeJS.ErrnoException)
  if (manifest instanceof Error || !manifest.isFile() || manifest.isSymbolicLink()) {
    return err({
      _tag: 'SourceUnavailable',
      message: 'Local package directory must contain package.json.',
      attempts: [{ source: packageDir, status: 'skipped', reason: 'package.json is unavailable.' }],
    })
  }

  const source = await collectHostDirectory(packageDir, policy, packageDir, signal)
  return source._tag === 'Err'
    ? source
    : ok({ skillName: 'generate-package-skill', outputName: input.destination.name, source: source.value, current: current.value, destination: input.destination })
}

function requestContent(skill: HarnessV1Skill): string {
  const request = skill.files?.find(file => file.path === 'assets/harness-request.md')
  if (!request)
    throw new Error(`Harness request asset is missing for ${skill.name}.`)
  return request.content
}

function renderRequest(template: string, sourcePath: string, currentSkillPath: string, outputPath: string, skillName: string): string {
  return template
    .replaceAll('{{SOURCE_PATH}}', sourcePath)
    .replaceAll('{{CURRENT_SKILL_PATH}}', currentSkillPath)
    .replaceAll('{{OUTPUT_PATH}}', outputPath)
    .replaceAll('{{SKILL_NAME}}', skillName)
}

async function writePreparedSource(active: ActiveSandbox, prepared: PreparedRun, signal?: AbortSignal): Promise<void> {
  const sourcePath = posix.join(active.workDir, 'input/source')
  const reset = await active.sandbox.run({
    command: 'rm -rf -- "$SKILLD_INPUT" "$SKILLD_OUTPUT" && mkdir -p -- "$SKILLD_INPUT"',
    env: {
      SKILLD_INPUT: posix.join(active.workDir, 'input'),
      SKILLD_OUTPUT: posix.join(active.workDir, 'skilld-output'),
    },
    abortSignal: signal,
  })
  if (reset.exitCode !== 0)
    throw new Error(reset.stderr.trim() || 'Harness work directory cannot be prepared.')
  for (const file of prepared.source.files) {
    await active.sandbox.writeBinaryFile({
      path: posix.join(sourcePath, file.path),
      content: file.content,
      abortSignal: signal,
    })
  }
  for (const file of prepared.current?.files ?? []) {
    await active.sandbox.writeBinaryFile({
      path: posix.join(active.workDir, 'input/current-skill', file.path),
      content: file.content,
      abortSignal: signal,
    })
  }
  await active.sandbox.writeTextFile({
    path: posix.join(active.workDir, 'input/source-manifest.json'),
    content: `${JSON.stringify({
      sourceAttempts: prepared.source.attempts,
      npmResolution: prepared.source.npmResolution,
      hasCurrentSkill: prepared.current !== undefined,
    }, null, 2)}\n`,
    abortSignal: signal,
  })
}

function toAgentError(cause: unknown, signal?: AbortSignal): SkillRunResult {
  return signal?.aborted
    ? cancelled()
    : err({ _tag: 'AgentFailed', message: 'Harness Agent failed during the Skill run.', cause })
}

export function createSkillHarness(options: CreateSkillHarnessOptions): SkillHarness {
  const policy = parseOutputPolicy({ ...DEFAULT_OUTPUT_POLICY, ...options.outputPolicy })
  const fetchClient: FetchClient = options.fetch ?? globalThis.fetch.bind(globalThis)
  const sandboxConfig = { ...options.sandboxConfig }
  const harness = options.harness
  const sandbox = options.sandbox

  return {
    run: async (input, runOptions = {}): Promise<SkillRunResult> => {
      const parsed = parseSkillRun(input)
      if (parsed._tag === 'Err')
        return parsed

      const prepared = await prepareRun(parsed.value, policy, fetchClient, runOptions.signal)
      if (prepared._tag === 'Err')
        return prepared

      const skill = await loadSkilldMaintainedSkill(prepared.value.skillName)
      let active: ActiveSandbox | undefined
      const userOnSession = sandboxConfig.onSession
      const agent = new HarnessAgent({
        harness,
        sandbox,
        skills: [skill],
        permissionMode: 'allow-all',
        sandboxConfig: {
          ...sandboxConfig,
          onSession: async (sessionOptions) => {
            active = { sandbox: sessionOptions.session, workDir: sessionOptions.sessionWorkDir }
            await writePreparedSource(active, prepared.value, runOptions.signal)
            await userOnSession?.(sessionOptions)
          },
        },
      })

      const sessionResult = await agent.createSession({ abortSignal: runOptions.signal }).then(ok, cause => err(cause))
      if (sessionResult._tag === 'Err')
        return toAgentError(sessionResult.error, runOptions.signal)
      const session = sessionResult.value

      try {
        if (!active)
          return err({ _tag: 'AgentFailed', message: 'Harness did not provide its sandbox session.' })
        const sourcePath = posix.join(active.workDir, 'input/source')
        const currentSkillPath = posix.join(active.workDir, 'input/current-skill')
        const outputPath = posix.join(active.workDir, 'skilld-output', prepared.value.outputName)
        const prompt = renderRequest(requestContent(skill), sourcePath, currentSkillPath, outputPath, prepared.value.outputName)
        const generated = await agent.generate({ session, prompt, abortSignal: runOptions.signal }).then(ok, cause => err(cause))
        if (generated._tag === 'Err')
          return toAgentError(generated.error, runOptions.signal)
        if (runOptions.signal?.aborted)
          return cancelled()

        const collected = await collectSandboxOutput(active.sandbox, outputPath, policy, runOptions.signal)
        if (collected._tag === 'Err')
          return collected

        if (prepared.value.skillName === 'review-skill')
          return validateSkillReview(collected.value)

        const validated = validateGeneratedSkill(prepared.value.outputName, collected.value)
        if (validated._tag === 'Err')
          return validated
        if (!prepared.value.destination)
          return err({ _tag: 'InvalidInput', message: 'Skill destination is required.' })
        return promoteSkill(
          prepared.value.destination.rootDir,
          prepared.value.destination.name,
          collected.value,
          prepared.value.source.attempts,
        )
      }
      finally {
        await session.destroy()
      }
    },
  }
}
