import type { HarnessV1NetworkSandboxSession, HarnessV1Skill } from '@ai-sdk/harness'
import type { CollectedFile, SandboxSession } from './internal/output/collect.ts'
import type { Result } from './internal/result.ts'
import type { PreparedSource } from './internal/source/host.ts'
import type { FetchClient } from './internal/source/npm.ts'
import type {
  CreateSkillHarnessOptions,
  GeneratedSkill,
  SkillDestination,
  SkillHarness,
  SkillOutputPolicy,
  SkillReview,
  SkillRun,
  SkillRunError,
  SkillRunEvent,
  SkillRunOptions,
  SkillRunReport,
  SkillRunResult,
  SkillRunUsage,
} from './types.ts'
import { lstat, realpath } from 'node:fs/promises'
import { basename, join, posix, resolve } from 'node:path'
import { HarnessAgent } from '@ai-sdk/harness/agent'
import { parseOutputPolicy, parseSkillRun } from './internal/input.ts'
import { collectSandboxOutput } from './internal/output/collect.ts'
import { promoteSkill } from './internal/output/promote.ts'
import { validateGeneratedSkill, validateSkillReview } from './internal/output/validate.ts'
import { resolveWithin } from './internal/paths.ts'
import { err, ok } from './internal/result.ts'
import { collectHostDirectory, skippedFileWarnings } from './internal/source/host.ts'
import { prepareNpmPackage } from './internal/source/npm.ts'
import { DEFAULT_OUTPUT_POLICY, loadSkilldMaintainedSkill } from './skills.ts'

interface PreparedRun {
  readonly skillName: 'generate-package-skill' | 'update-package-skill' | 'generate-project-skill' | 'review-skill'
  readonly outputName: string
  readonly source: PreparedSource
  // Path under `input/source`. A reviewed Skill keeps its directory name, so
  // review-skill can compare that name with the frontmatter `name`.
  readonly sourceDir: string
  readonly current?: PreparedSource
  readonly destination?: SkillDestination
}

interface ActiveSandbox {
  readonly sandbox: SandboxSession
  readonly workDir: string
}

/** The outcome of one Skill run before the Harness attaches its report. */
type RunOutcome = Result<GeneratedSkill | SkillReview, SkillRunError>

/** Collects the cost of one Skill run and every warning beside its outcome. */
interface RunLog {
  /** Forwards an event to the caller and counts each finished step. */
  readonly emit: (event: SkillRunEvent) => void
  readonly warn: (warning: string) => void
  /** Replaces the counted steps with the totals the Agent reported at its end. */
  readonly settle: (usage: SkillRunUsage, steps: number) => void
  readonly report: () => SkillRunReport
}

function addCount(total: number | undefined, count: number | undefined): number | undefined {
  return total === undefined ? count : count === undefined ? total : total + count
}

function isPromiseLike(value: unknown): value is PromiseLike<unknown> {
  return (typeof value === 'object' || typeof value === 'function')
    && value !== null
    && typeof (value as { then?: unknown }).then === 'function'
}

function createRunLog(onEvent: SkillRunOptions['onEvent']): RunLog {
  const warnings: string[] = []
  const eventFailures = new Set<string>()
  let usage: SkillRunUsage = { inputTokens: undefined, cachedInputTokens: undefined, outputTokens: undefined }
  let steps = 0
  // The Agent SDK drops callback errors, so surface them on the report instead.
  const fail = (cause: unknown) => {
    eventFailures.add(`onEvent failed: ${cause instanceof Error ? cause.message : String(cause)}`)
  }
  return {
    emit: (event) => {
      if (event._tag === 'StepFinish') {
        steps += 1
        usage = {
          inputTokens: addCount(usage.inputTokens, event.usage.inputTokens),
          cachedInputTokens: addCount(usage.cachedInputTokens, event.usage.cachedInputTokens),
          outputTokens: addCount(usage.outputTokens, event.usage.outputTokens),
        }
      }
      if (!onEvent)
        return
      try {
        const returned: unknown = onEvent(event)
        // An async listener or custom thenable would otherwise reject unhandled and can end the host process.
        if (isPromiseLike(returned))
          Promise.resolve(returned).catch(fail)
      }
      catch (cause) {
        fail(cause)
      }
    },
    warn: warning => warnings.push(warning),
    settle: (total, count) => {
      usage = total
      steps = count
    },
    report: () => ({ usage, steps, warnings: [...warnings, ...eventFailures] }),
  }
}

/** The fields this package reads from the Agent SDK usage report. */
interface AgentUsage {
  readonly inputTokens: number | undefined
  readonly inputTokenDetails: { readonly cacheReadTokens: number | undefined }
  readonly outputTokens: number | undefined
}

function toUsage(usage: AgentUsage): SkillRunUsage {
  return {
    inputTokens: usage.inputTokens,
    cachedInputTokens: usage.inputTokenDetails.cacheReadTokens,
    outputTokens: usage.outputTokens,
  }
}

/** An unknown count in any turn keeps the total unknown. */
function sumUsage(a: SkillRunUsage, b: SkillRunUsage): SkillRunUsage {
  const sum = (x: number | undefined, y: number | undefined) => x === undefined || y === undefined ? undefined : x + y
  return { inputTokens: sum(a.inputTokens, b.inputTokens), cachedInputTokens: sum(a.cachedInputTokens, b.cachedInputTokens), outputTokens: sum(a.outputTokens, b.outputTokens) }
}

/** Turns the Agent gets to fix output that failed the deterministic checks, in the same session. */
const OUTPUT_REPAIR_TURNS = 2

function repairRequest(outputPath: string, issues: ReadonlyArray<string>): string {
  return [
    `The Harness checked the output at \`${outputPath}\` and found these problems:`,
    '',
    ...issues.map(issue => `- ${issue}`),
    '',
    `Fix each problem in place. Write no files outside \`${outputPath}\`. Finish when the output passes these checks.`,
  ].join('\n')
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
      : ok({ skillName: 'generate-project-skill', outputName: input.destination.name, source: source.value, sourceDir: '', current: current.value, destination: input.destination })
  }

  if (input._tag === 'ReviewSkill') {
    const source = await collectHostDirectory(input.skillDir, policy, input.skillDir, signal)
    return source._tag === 'Err'
      ? source
      : ok({ skillName: 'review-skill', outputName: 'review', source: source.value, sourceDir: basename(input.skillDir) })
  }

  if (input.source._tag === 'NpmPackage') {
    const source = await prepareNpmPackage(input.source.spec, policy, fetchClient, signal)
    if (signal?.aborted)
      return err({ _tag: 'Cancelled', message: 'Skill run was cancelled.' })
    return source._tag === 'Err'
      ? source
      : ok({ skillName: packageSkillName(current.value), outputName: input.destination.name, source: source.value, sourceDir: '', current: current.value, destination: input.destination })
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
    : ok({ skillName: packageSkillName(current.value), outputName: input.destination.name, source: source.value, sourceDir: '', current: current.value, destination: input.destination })
}

/** A destination that already holds the Skill gets an update run, which tests only what the release changed. */
function packageSkillName(current: PreparedSource | undefined): 'generate-package-skill' | 'update-package-skill' {
  return current === undefined ? 'generate-package-skill' : 'update-package-skill'
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

function sourcePathOf(active: ActiveSandbox, prepared: PreparedRun): string {
  return posix.join(active.workDir, 'input/source', prepared.sourceDir)
}

async function writePreparedSource(active: ActiveSandbox, prepared: PreparedRun, signal?: AbortSignal): Promise<void> {
  const sourcePath = sourcePathOf(active, prepared)
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
      skippedFiles: prepared.source.skippedFiles,
      npmResolution: prepared.source.npmResolution,
      hasCurrentSkill: prepared.current !== undefined,
    }, null, 2)}\n`,
    abortSignal: signal,
  })
}

function agentError(cause: unknown, signal?: AbortSignal): SkillRunError {
  return signal?.aborted
    ? { _tag: 'Cancelled', message: 'Skill run was cancelled.' }
    : { _tag: 'AgentFailed', message: 'Harness Agent failed during the Skill run.', cause }
}

function toAgentError(cause: unknown, signal?: AbortSignal): RunOutcome {
  return err(agentError(cause, signal))
}

export function createSkillHarness(options: CreateSkillHarnessOptions): SkillHarness {
  const policy = parseOutputPolicy({ ...DEFAULT_OUTPUT_POLICY, ...options.outputPolicy })
  const fetchClient: FetchClient = options.fetch ?? globalThis.fetch.bind(globalThis)
  const sandboxConfig = { ...options.sandboxConfig }
  const harness = options.harness
  const sandbox = options.sandbox

  return {
    // Each input tag maps to one Skill, and that Skill decides the value tag,
    // so the wide result is the narrowed result for this input.
    run: <Run extends SkillRun>(input: Run, runOptions: SkillRunOptions = {}) =>
      runSkill(input, runOptions) as Promise<SkillRunResult<Run['_tag']>>,
  }

  async function runSkill(input: SkillRun, runOptions: SkillRunOptions): Promise<SkillRunResult> {
    const log = createRunLog(runOptions.onEvent)
    const outcome = await runLogged(input, runOptions, log)
    const report = log.report()
    return outcome._tag === 'Ok'
      ? { _tag: 'Ok', value: outcome.value, report }
      : { _tag: 'Err', error: outcome.error, report }
  }

  async function runLogged(input: SkillRun, runOptions: SkillRunOptions, log: RunLog): Promise<RunOutcome> {
    const parsed = parseSkillRun(input)
    if (parsed._tag === 'Err')
      return parsed

    const prepared = await prepareRun(parsed.value, policy, fetchClient, runOptions.signal)
    if (prepared._tag === 'Err')
      return prepared
    for (const warning of skippedFileWarnings(prepared.value.source.skippedFiles, policy))
      log.warn(warning)

    const skill = await loadSkilldMaintainedSkill(prepared.value.skillName)
    // The Harness creates and owns the sandbox session, and hands it to the
    // Agent. The Agent never sees the provider, so it never destroys the
    // session itself, and the deprecated provider path stays unused.
    const sandboxSession = await sandbox.createSession({ abortSignal: runOptions.signal }).then(ok, cause => err(cause))
    if (sandboxSession._tag === 'Err')
      return toAgentError(sandboxSession.error, runOptions.signal)
    const result = await runAgent(prepared.value, skill, sandboxSession.value, log, runOptions.signal)
    const destroyed = await Promise.resolve(sandboxSession.value.destroy()).then(() => undefined, cause => cause as unknown)
    if (destroyed !== undefined)
      log.warn(`Sandbox session cleanup failed: ${destroyed instanceof Error ? destroyed.message : String(destroyed)}`)
    return result
  }

  async function runAgent(
    prepared: PreparedRun,
    skill: HarnessV1Skill,
    sandboxSession: HarnessV1NetworkSandboxSession,
    log: RunLog,
    signal?: AbortSignal,
  ): Promise<RunOutcome> {
    let active: ActiveSandbox | undefined
    const userOnSession = sandboxConfig.onSession
    const agent = new HarnessAgent({
      harness,
      skills: [skill],
      permissionMode: 'allow-all',
      sandboxConfig: {
        ...sandboxConfig,
        onSession: async (sessionOptions) => {
          active = { sandbox: sessionOptions.session, workDir: sessionOptions.sessionWorkDir }
          await writePreparedSource(active, prepared, signal)
          await userOnSession?.(sessionOptions)
        },
      },
    })

    const sessionResult = await agent.createSession({ sandboxSession, abortSignal: signal }).then(ok, cause => err(cause))
    if (sessionResult._tag === 'Err')
      return toAgentError(sessionResult.error, signal)
    const session = sessionResult.value

    try {
      if (!active)
        return err({ _tag: 'AgentFailed', message: 'Harness did not provide its sandbox session.' })
      const sourcePath = sourcePathOf(active, prepared)
      const currentSkillPath = posix.join(active.workDir, 'input/current-skill')
      const outputPath = posix.join(active.workDir, 'skilld-output', prepared.outputName)
      const prompt = renderRequest(requestContent(skill), sourcePath, currentSkillPath, outputPath, prepared.outputName)
      const outputSandbox = active.sandbox
      // Steps count across turns, so a repair turn continues the numbering.
      let step = 0
      let stepOffset = 0
      let usage: SkillRunUsage | undefined
      const turn = async (text: string): Promise<Result<void, SkillRunError>> => {
        const generated = await agent.generate({
          session,
          prompt: text,
          abortSignal: signal,
          onStepStart: (event) => {
            step = stepOffset + event.stepNumber
            log.emit({ _tag: 'StepStart', step })
          },
          onToolExecutionStart: ({ toolCall }) => {
            log.emit({ _tag: 'ToolCall', step, toolName: toolCall.toolName, toolCallId: toolCall.toolCallId, input: toolCall.input })
          },
          onStepEnd: (result) => {
            log.emit({ _tag: 'StepFinish', step: stepOffset + result.stepNumber, finishReason: result.finishReason, usage: toUsage(result.usage) })
          },
        }).then(ok, cause => err(cause))
        if (generated._tag === 'Err')
          return err(agentError(generated.error, signal))
        const turnUsage = toUsage(generated.value.totalUsage)
        usage = usage ? sumUsage(usage, turnUsage) : turnUsage
        stepOffset += generated.value.steps.length
        log.settle(usage, stepOffset)
        return signal?.aborted ? err({ _tag: 'Cancelled', message: 'Skill run was cancelled.' }) : ok(undefined)
      }
      const check = async (): Promise<Result<ReadonlyArray<CollectedFile>, SkillRunError>> => {
        const collected = await collectSandboxOutput(outputSandbox, outputPath, policy, signal)
        if (collected._tag === 'Err')
          return collected
        const validated = prepared.skillName === 'review-skill'
          ? validateSkillReview(collected.value)
          : validateGeneratedSkill(
              prepared.outputName,
              collected.value,
              prepared.skillName === 'generate-project-skill'
                ? { _tag: 'ProjectSkill', projectPaths: prepared.source.files.map(file => file.path) }
                : { _tag: 'PackageSkill' },
            )
        return validated._tag === 'Err' ? validated : ok(collected.value)
      }

      const first = await turn(prompt)
      if (first._tag === 'Err')
        return first
      let checked = await check()
      // A failed check goes back to the Agent, which still holds its context, instead of failing the run.
      for (let repair = 1; repair <= OUTPUT_REPAIR_TURNS && checked._tag === 'Err' && checked.error._tag === 'InvalidSkill'; repair++) {
        log.warn(`Output checks failed; repair turn ${repair} of ${OUTPUT_REPAIR_TURNS}: ${checked.error.issues.join(' ')}`)
        const repaired = await turn(repairRequest(outputPath, checked.error.issues))
        if (repaired._tag === 'Err')
          return repaired
        checked = await check()
      }
      if (checked._tag === 'Err')
        return checked
      const collected = checked

      if (prepared.skillName === 'review-skill')
        return validateSkillReview(collected.value)

      if (!prepared.destination)
        return err({ _tag: 'InvalidInput', message: 'Skill destination is required.' })
      const promoted = await promoteSkill(
        prepared.destination.rootDir,
        prepared.destination.name,
        collected.value,
        prepared.source.attempts,
      )
      if (promoted._tag === 'Err')
        return promoted
      for (const warning of promoted.value.warnings)
        log.warn(warning)
      return ok(promoted.value.skill)
    }
    finally {
      await session.destroy()
    }
  }
}
