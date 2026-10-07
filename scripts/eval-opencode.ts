// Runs the eval cases with opencode on a cheap model. Each case runs once with
// the skilld-maintained Skills and once without them; the delta is what the
// Skills add. The friction report lists failed commands and skilld errors, so
// a run also shows where the Skill, the CLI, the API, or the MCP server
// confused the Agent.
//
// Usage: node scripts/eval-opencode.ts [--case <glob>] [--arm with|without|both]
//   [--runs N] [--model provider/model] [--judge-model provider/model] [--no-llm]
//   [--site https://preview.example] [--cli path/to/skilld] [--concurrency N]
//   [--timeout seconds] [--keep-temp]
import type { EvalCase } from './eval-opencode/case.ts'
import type { GradedGrader, JudgeVerdict } from './eval-opencode/grade.ts'
import type { Friction, Transcript } from './eval-opencode/transcript.ts'
import { spawn, spawnSync } from 'node:child_process'
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { parseArgs } from 'node:util'
import { findCaseDirs, loadCase } from './eval-opencode/case.ts'
import { runEnv } from './eval-opencode/env.ts'
import { gradeRun, parseVerdict, runScore } from './eval-opencode/grade.ts'
import { friction, parseTranscript } from './eval-opencode/transcript.ts'

const root = resolve(dirname(new URL(import.meta.url).pathname), '..')
const CASE_ROOTS = [join(root, 'evals'), join(root, 'evals-opencode')]
const DEFAULT_MODEL = 'zai-coding-plan/glm-5.3-flash'
const DEFAULT_SITE = 'https://skilld.dev'
const JUDGE_INPUT_CHARS = 30_000

type ArmName = 'with' | 'without'

interface Options {
  caseGlob: string | null
  arms: ArmName[]
  runs: number
  model: string
  judgeModel: string
  llm: boolean
  site: string
  cli: string
  concurrency: number
  timeoutMs: number
  keepTemp: boolean
}

interface RunResult {
  caseName: string
  arm: ArmName
  run: number
  exit: { _tag: 'Exited', code: number | null } | { _tag: 'TimedOut' }
  seconds: number
  score: number | null
  graded: GradedGrader[]
  friction: Friction
  tokens: number
  lastMessage: string
}

function fail(message: string): never {
  console.error(message)
  process.exit(1)
}

function readOptions(): Options {
  const { values } = parseArgs({
    options: {
      'case': { type: 'string' },
      'arm': { type: 'string', default: 'both' },
      'runs': { type: 'string', default: '1' },
      'model': { type: 'string', default: DEFAULT_MODEL },
      'judge-model': { type: 'string' },
      'no-llm': { type: 'boolean', default: false },
      'site': { type: 'string', default: DEFAULT_SITE },
      'cli': { type: 'string' },
      'concurrency': { type: 'string', default: '2' },
      'timeout': { type: 'string', default: '600' },
      'keep-temp': { type: 'boolean', default: false },
    },
  })
  const arm = values.arm
  if (arm !== 'with' && arm !== 'without' && arm !== 'both')
    fail(`--arm must be with, without, or both. Received ${arm}.`)
  const cli = values.cli ? resolve(values.cli) : whichSkilld()
  if (!cli)
    fail('skilld is not on PATH. Pass --cli with the path to a skilld executable.')
  return {
    caseGlob: values.case ?? null,
    arms: arm === 'both' ? ['with', 'without'] : [arm],
    runs: Math.max(1, Number(values.runs) || 1),
    model: values.model,
    judgeModel: values['judge-model'] ?? values.model,
    llm: !values['no-llm'],
    site: values.site.replace(/\/$/, ''),
    cli,
    concurrency: Math.max(1, Number(values.concurrency) || 1),
    timeoutMs: Math.max(30, Number(values.timeout) || 600) * 1000,
    keepTemp: values['keep-temp'],
  }
}

function whichSkilld(): string | null {
  const found = spawnSync('which', ['skilld'], { encoding: 'utf8' })
  return found.status === 0 ? found.stdout.trim() : null
}

function globRegex(glob: string): RegExp {
  return new RegExp(`^${glob.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.')}$`)
}

function isolatedEnv(home: string, config: Record<string, unknown>, extra: Record<string, string>): NodeJS.ProcessEnv {
  const auth = join(process.env.HOME ?? '', '.local/share/opencode/auth.json')
  return runEnv({ base: process.env, tempHome: home, config, auth: existsSync(auth) ? readFileSync(auth, 'utf8') : null, extra })
}

function userProviders(): Record<string, unknown> {
  const file = join(process.env.XDG_CONFIG_HOME ?? join(process.env.HOME ?? '', '.config'), 'opencode/opencode.json')
  if (!existsSync(file))
    return {}
  const config = JSON.parse(readFileSync(file, 'utf8')) as Record<string, unknown>
  return typeof config.provider === 'object' && config.provider !== null ? config.provider as Record<string, unknown> : {}
}

/** Replaces `{site}` in a case's opencode config, so its MCP server can point at a preview. */
function withSite(value: unknown, site: string): unknown {
  if (typeof value === 'string')
    return value.replaceAll('{site}', site)
  if (Array.isArray(value))
    return value.map(v => withSite(v, site))
  if (typeof value === 'object' && value !== null)
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, withSite(v, site)]))
  return value
}

function runOpencode(args: { dir: string, prompt: string, model: string, env: NodeJS.ProcessEnv, timeoutMs: number }): Promise<{ stdout: string, stderr: string, exit: RunResult['exit'] }> {
  return new Promise((done) => {
    const child = spawn('opencode', ['run', '--format', 'json', '--dir', args.dir, '--auto', '--pure', '-m', args.model, args.prompt], { env: args.env, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    let timedOut = false
    const timer = setTimeout(() => {
      timedOut = true
      child.kill('SIGTERM')
    }, args.timeoutMs)
    child.stdout.on('data', (chunk) => { stdout += chunk })
    child.stderr.on('data', (chunk) => { stderr += chunk })
    child.on('close', (code) => {
      clearTimeout(timer)
      done({ stdout, stderr, exit: timedOut ? { _tag: 'TimedOut' } : { _tag: 'Exited', code } })
    })
  })
}

const SKILL_NAMES: string[] = JSON.parse(readFileSync(join(root, 'skills/skilld-maintained-skills.json'), 'utf8'))

async function judge(criteria: string, content: string, opts: Options, tmpRoot: string): Promise<JudgeVerdict | string> {
  const home = mkdtempSync(join(tmpRoot, 'judge-'))
  const project = join(home, 'project')
  mkdirSync(project)
  const config = { $schema: 'https://opencode.ai/config.json', provider: userProviders(), autoupdate: false, share: 'disabled', agent: { build: { steps: 2 } } }
  const prompt = [
    'You grade one run of a coding agent. Use no tools.',
    'Reply with only one JSON object: {"pass": true or false, "reason": "one sentence"}.',
    '',
    'Criteria:',
    criteria,
    '',
    'Run output:',
    '<<<',
    content.length > JUDGE_INPUT_CHARS ? `${content.slice(0, JUDGE_INPUT_CHARS)}\n[truncated]` : content,
    '>>>',
  ].join('\n')
  const result = await runOpencode({ dir: project, prompt, model: opts.judgeModel, env: isolatedEnv(home, config, {}), timeoutMs: 180_000 })
  if (!opts.keepTemp)
    rmSync(home, { recursive: true, force: true })
  const verdict = parseVerdict(parseTranscript(result.stdout).lastMessage)
  return verdict ?? `judge reply was not a verdict (exit ${result.exit._tag === 'Exited' ? result.exit.code : 'timeout'})`
}

async function runCase(evalCase: EvalCase, arm: ArmName, run: number, opts: Options, tmpRoot: string, outDir: string): Promise<RunResult> {
  const home = mkdtempSync(join(tmpRoot, `${evalCase.name}-${arm}-${run}-`))
  const project = join(home, 'project')
  const bin = join(home, 'bin')
  mkdirSync(project, { recursive: true })
  mkdirSync(bin)
  symlinkSync(opts.cli, join(bin, 'skilld'))

  if (evalCase.scaffoldScript) {
    const scaffold = spawnSync('bash', [evalCase.scaffoldScript], { cwd: project, encoding: 'utf8' })
    if (scaffold.status !== 0)
      fail(`${evalCase.name}: scaffold failed\n${scaffold.stderr}`)
  }
  if (arm === 'with') {
    for (const name of SKILL_NAMES)
      cpSync(join(root, 'skills', name), join(project, '.opencode/skills', name), { recursive: true })
  }

  const config = {
    $schema: 'https://opencode.ai/config.json',
    provider: userProviders(),
    autoupdate: false,
    share: 'disabled',
    agent: { build: { steps: evalCase.maxTurns } },
    ...(withSite(evalCase.opencodeConfig ?? {}, opts.site) as Record<string, unknown>),
  }
  const env = isolatedEnv(home, config, {
    PATH: `${bin}:${process.env.PATH ?? ''}`,
    SKILLD_DATA_DIR: join(home, 'skilld'),
    ...(opts.site === DEFAULT_SITE ? {} : { SKILLD_API_URL: opts.site }),
  })

  const started = Date.now()
  const result = await runOpencode({ dir: project, prompt: evalCase.prompt, model: opts.model, env, timeoutMs: opts.timeoutMs })
  const seconds = Math.round((Date.now() - started) / 1000)
  const transcript: Transcript = parseTranscript(result.stdout)

  const verdicts = new Map<string, JudgeVerdict>()
  const judgeNotes = new Map<string, string>()
  if (opts.llm) {
    for (const grader of evalCase.graders) {
      if (grader._tag !== 'llm' || (grader.arm === 'with-only' && arm !== 'with'))
        continue
      const content = grader.focus === 'trace' ? transcript.trace : transcript.lastMessage
      const verdict = await judge(grader.criteria, content, opts, tmpRoot)
      if (typeof verdict === 'string')
        judgeNotes.set(grader.name, verdict)
      else verdicts.set(grader.name, verdict)
    }
  }

  const readProjectFile = (path: string) => {
    const file = join(project, path)
    return existsSync(file) ? readFileSync(file, 'utf8') : null
  }
  const graded = gradeRun(evalCase.graders, { transcript, readProjectFile, verdicts, arm }).map(g =>
    g.outcome._tag === 'Skipped' && judgeNotes.has(g.name) ? { ...g, outcome: { _tag: 'Skipped' as const, reason: judgeNotes.get(g.name)! } } : g,
  )

  const runDir = join(outDir, evalCase.name, `${arm}-${run}`)
  mkdirSync(runDir, { recursive: true })
  writeFileSync(join(runDir, 'events.jsonl'), result.stdout)
  writeFileSync(join(runDir, 'stderr.log'), result.stderr)
  const runResult: RunResult = {
    caseName: evalCase.name,
    arm,
    run,
    exit: result.exit,
    seconds,
    score: runScore(graded),
    graded,
    friction: friction(transcript),
    tokens: transcript.tokens,
    lastMessage: transcript.lastMessage,
  }
  writeFileSync(join(runDir, 'run.json'), `${JSON.stringify(runResult, null, 2)}\n`)
  if (!opts.keepTemp)
    rmSync(home, { recursive: true, force: true })
  console.error(`${evalCase.name} ${arm} #${run}: ${runResult.score === null ? 'no score' : runResult.score.toFixed(2)} in ${seconds}s`)
  return runResult
}

async function pool<T, R>(items: T[], size: number, work: (item: T) => Promise<R>): Promise<R[]> {
  const results: R[] = Array.from({ length: items.length })
  let next = 0
  await Promise.all(Array.from({ length: Math.min(size, items.length) }, async () => {
    while (next < items.length) {
      const index = next++
      results[index] = await work(items[index]!)
    }
  }))
  return results
}

function mean(values: (number | null)[]): number | null {
  const scored = values.filter((v): v is number => v !== null)
  return scored.length === 0 ? null : scored.reduce((a, b) => a + b, 0) / scored.length
}

function fixed(value: number | null): string {
  return value === null ? 'n/a' : value.toFixed(2)
}

function report(cases: EvalCase[], results: RunResult[], opts: Options, header: string[]): string {
  const lines = [...header, '', '| Case | With | Without | Δ | Skill fired |', '| --- | --- | --- | --- | --- |']
  for (const evalCase of cases) {
    const of = (arm: ArmName) => results.filter(r => r.caseName === evalCase.name && r.arm === arm)
    const withScore = mean(of('with').map(r => r.score))
    const withoutScore = mean(of('without').map(r => r.score))
    const delta = withScore !== null && withoutScore !== null ? withScore - withoutScore : null
    const indicators = of('with').flatMap(r => r.graded.filter(g => g.indicator))
    const fired = indicators.filter(g => g.outcome._tag === 'Scored' && g.outcome.score === 1).length
    lines.push(`| \`${evalCase.name}\` | ${fixed(withScore)} | ${fixed(withoutScore)} | ${delta === null ? 'n/a' : `${delta >= 0 ? '+' : ''}${delta.toFixed(2)}`} | ${indicators.length ? `${fired}/${indicators.length}` : 'n/a'} |`)
  }

  lines.push('', '## Graders')
  for (const r of results) {
    const failed = r.graded.filter(g => g.outcome._tag !== 'Scored' || g.outcome.score < 1)
    if (failed.length === 0)
      continue
    lines.push('', `### ${r.caseName}, ${r.arm} #${r.run}`)
    for (const g of failed)
      lines.push(`- ${g.name}${g.indicator ? ' (indicator)' : ''}: ${g.outcome._tag === 'Scored' ? `failed, ${g.outcome.detail}` : `skipped, ${g.outcome.reason}`}`)
  }

  lines.push('', '## Friction')
  for (const r of results) {
    const f = r.friction
    const entries = [
      ...(r.exit._tag === 'TimedOut' ? [`- Run timed out after ${opts.timeoutMs / 1000}s`] : []),
      ...f.failedCommands.map(c => `- Command failed (exit ${c.exit ?? 'unknown'}): \`${c.command}\``),
      ...f.skilldErrors.map(e => `- skilld error \`${e.code}\`: ${e.message}${e.nextStep ? ` Next step: ${e.nextStep}` : ' (no next step)'}`),
      ...f.nextSteps.map(s => `- Next step printed: ${s}`),
      ...f.repeatedCommands.map(c => `- Ran ${c.times} times: \`${c.command}\``),
      ...f.toolErrors.map(e => `- Tool \`${e.tool}\` failed: ${e.error}`),
    ]
    if (entries.length)
      lines.push('', `### ${r.caseName}, ${r.arm} #${r.run}`, ...entries)
  }
  return `${lines.join('\n')}\n`
}

async function main() {
  const opts = readOptions()
  const filter = opts.caseGlob ? globRegex(opts.caseGlob) : null
  const cases = CASE_ROOTS.flatMap(findCaseDirs).map((dir) => {
    const loaded = loadCase(dir)
    return loaded._tag === 'Ok' ? loaded.value : fail(loaded.message)
  }).filter(c => !filter || filter.test(c.name))
  if (cases.length === 0)
    fail('No eval case matches.')

  const stamp = new Date().toISOString().replace(/[:.]/g, '-')
  const outDir = join(root, 'evals/results/opencode', stamp)
  const tmpRoot = mkdtempSync(join(tmpdir(), 'skilld-opencode-eval-'))
  mkdirSync(outDir, { recursive: true })

  const cliVersion = spawnSync(opts.cli, ['--version'], { encoding: 'utf8' }).stdout.trim()
  const opencodeVersion = spawnSync('opencode', ['--version'], { encoding: 'utf8' }).stdout.trim()
  const jobs = cases.flatMap(c => opts.arms.flatMap(arm => Array.from({ length: opts.runs }, (_, i) => ({ c, arm, run: i + 1 }))))
  console.error(`${jobs.length} runs on ${opts.model}, ${opts.concurrency} at a time. Results: ${outDir}`)

  const started = Date.now()
  const results = await pool(jobs, opts.concurrency, job => runCase(job.c, job.arm, job.run, opts, tmpRoot, outDir))
  const minutes = ((Date.now() - started) / 60_000).toFixed(1)
  if (!opts.keepTemp)
    rmSync(tmpRoot, { recursive: true, force: true })

  const text = report(cases, results, opts, [
    `# opencode evals, ${stamp}`,
    '',
    `Model ${opts.model}, judge ${opts.llm ? opts.judgeModel : 'off'}, ${opts.runs} run per arm, ${minutes} min wall time.`,
    `${cliVersion} at ${opts.cli}. opencode ${opencodeVersion}. Site ${opts.site}.`,
    `Tokens: ${results.reduce((sum, r) => sum + r.tokens, 0)}.`,
  ])
  writeFileSync(join(outDir, 'report.md'), text)
  writeFileSync(join(outDir, 'summary.json'), `${JSON.stringify(results, null, 2)}\n`)
  console.log(text)
}

await main()
