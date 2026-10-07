import type { Grader } from '../../scripts/eval-opencode/case.ts'
import { describe, expect, it } from 'vitest'
import { parseGrader } from '../../scripts/eval-opencode/case.ts'
import { runEnv } from '../../scripts/eval-opencode/env.ts'
import { gradeRun, opencodeToolName, parseVerdict, runScore } from '../../scripts/eval-opencode/grade.ts'
import { friction, parseTranscript } from '../../scripts/eval-opencode/transcript.ts'

function event(type: string, part: Record<string, unknown>) {
  return JSON.stringify({ type, part })
}

function bash(command: string, output: string, exit: number) {
  return event('tool_use', { tool: 'bash', state: { status: 'completed', input: { command }, output, metadata: { exit } } })
}

const missingRef = '{"schemaVersion":1,"_tag":"OperationError","error":{"code":"SOURCE_NOT_FOUND","message":"the Resolution failed"}}'

const stream = [
  event('step_start', {}),
  event('text', { text: 'Loading the skill.' }),
  event('tool_use', { tool: 'skill', state: { status: 'completed', input: { name: 'skilld' }, output: '<skill_content>' } }),
  event('step_finish', { tokens: { total: 100 }, cost: 0 }),
  event('step_start', {}),
  bash('skilld run a/b/c --json', missingRef, 1),
  bash('skilld run a/b/c --json', `${missingRef}\nNext step: search for another Skill.`, 1),
  event('tool_use', { tool: 'skilld_search_skills', state: { status: 'error', input: { query: 'x' }, error: 'Origin not allowed' } }),
  event('step_finish', { tokens: { total: 50 }, cost: 0 }),
  event('step_start', {}),
  event('text', { text: 'The Skill was not found: SOURCE_NOT_FOUND.' }),
  event('step_finish', { tokens: { total: 25 }, cost: 0 }),
  'opencode crashed',
].join('\n')

function grader(source: string): Grader {
  return parseGrader('g', source)
}

describe('parseTranscript', () => {
  it('keeps the last step text as the closing message and sums tokens', () => {
    const t = parseTranscript(stream)
    expect(t.lastMessage).toBe('The Skill was not found: SOURCE_NOT_FOUND.')
    expect(t.tokens).toBe(175)
    expect(t.toolCalls.map(c => c.tool)).toEqual(['skill', 'bash', 'bash', 'skilld_search_skills'])
    expect(t.stray).toEqual(['opencode crashed'])
  })
})

describe('friction', () => {
  it('reports failed commands, skilld error codes, next steps, retries, and tool errors', () => {
    const f = friction(parseTranscript(stream))
    expect(f.failedCommands).toHaveLength(2)
    expect(f.skilldErrors).toEqual([
      { code: 'SOURCE_NOT_FOUND', message: 'the Resolution failed', nextStep: null },
      { code: 'SOURCE_NOT_FOUND', message: 'the Resolution failed', nextStep: null },
    ])
    expect(f.nextSteps).toEqual(['search for another Skill.'])
    expect(f.repeatedCommands).toEqual([{ command: 'skilld run a/b/c --json', times: 2 }])
    expect(f.toolErrors).toEqual([{ tool: 'skilld_search_skills', error: 'Origin not allowed' }])
  })
})

describe('gradeRun', () => {
  const transcript = parseTranscript(stream)
  const ctx = { transcript, readProjectFile: (path: string) => path === 'present.md' ? 'hello' : null, verdicts: new Map(), arm: 'with' as const }

  it('matches shell input with input_match and honours max: 0', () => {
    const graded = gradeRun([
      grader('---\ntype: tool_used\ntool: Bash\ninput_match: "skilld run a/b/c"\nmin: 2\n---'),
      grader('---\ntype: tool_used\ntool: Bash\ninput_match: "skilld install "\nmax: 0\n---'),
      grader('---\ntype: tool_used\ntool: Bash\ninput_match: "skilld run "\nmax: 0\n---'),
    ], ctx)
    expect(graded.map(g => g.outcome._tag === 'Scored' && g.outcome.score)).toEqual([1, 1, 0])
  })

  it('maps Claude Code tool names, including MCP tools', () => {
    expect(opencodeToolName('Skill')).toBe('skill')
    expect(opencodeToolName('mcp__skilld__search_skills')).toBe('skilld_search_skills')
    const graded = gradeRun([grader('---\ntype: tool_used\ntool: mcp__skilld__search_skills\n---')], ctx)
    expect(graded[0]!.outcome).toMatchObject({ _tag: 'Scored', score: 1 })
  })

  it('scores regex targets and file_exists against the project', () => {
    const graded = gradeRun([
      grader('---\ntype: regex\npattern: "source_not_found"\nflags: i\n---'),
      grader('---\ntype: regex\npattern: "Loading"\nmatch: not_contains\n---'),
      grader('---\ntype: regex\ntarget: trace\npattern: "Origin not allowed"\n---'),
      grader('---\ntype: file_exists\npath: present.md\n---'),
      grader('---\ntype: file_exists\npath: absent.md\nexists: false\n---'),
    ], ctx)
    expect(graded.map(g => g.outcome._tag === 'Scored' && g.outcome.score)).toEqual([1, 1, 1, 1, 1])
  })

  it('keeps with-only graders out of the score and out of the without arm', () => {
    const graders = [
      grader('---\ntype: regex\npattern: "nothing like this"\nweight: 3\n---'),
      grader('---\ntype: regex\npattern: "SOURCE_NOT_FOUND"\n---'),
      grader('---\ntype: tool_used\ntool: Skill\narm: with-only\n---'),
    ]
    const withArm = gradeRun(graders, ctx)
    expect(withArm[2]!.indicator).toBe(true)
    expect(runScore(withArm)).toBe(0.25)
    expect(gradeRun(graders, { ...ctx, arm: 'without' })).toHaveLength(2)
  })

  it('skips an LLM grader without a verdict and an unknown grader type', () => {
    const graded = gradeRun([
      grader('---\ntype: llm\n---\nThe run reports the error.'),
      grader('---\ntype: baseline\n---'),
      grader('---\ntype: regex\nmatch: count:2\npattern: x\n---'),
    ], ctx)
    expect(graded.map(g => g.outcome._tag)).toEqual(['Skipped', 'Skipped', 'Skipped'])
    expect(runScore(graded)).toBeNull()
  })
})

describe('parseVerdict', () => {
  it('reads the JSON object from a judge reply', () => {
    expect(parseVerdict('Sure.\n{"pass": true, "reason": "Quotes the code."}')).toEqual({ pass: true, reason: 'Quotes the code.' })
    expect(parseVerdict('{"pass": "yes"}')).toBeNull()
    expect(parseVerdict('no json here')).toBeNull()
  })
})

describe('runEnv', () => {
  const input = {
    base: { HOME: '/home/dev', PATH: '/usr/bin', OPENCODE_CONFIG: '/home/dev/oc.json', CARGO_HOME: '/opt/cargo' },
    tempHome: '/tmp/run',
    config: { model: 'm' },
    auth: '{"provider":{}}',
    extra: { SKILLD_DATA_DIR: '/tmp/run/skilld' },
  }

  it('moves HOME and the XDG homes into the run directory', () => {
    const env = runEnv(input)
    expect(env.HOME).toBe('/tmp/run')
    expect(env.XDG_CONFIG_HOME).toBe('/tmp/run/config')
    expect(env.XDG_DATA_HOME).toBe('/tmp/run/data')
    expect(env.OPENCODE_CONFIG).toBeUndefined()
    expect(env.OPENCODE_DISABLE_CLAUDE_CODE).toBe('1')
    expect(env.SKILLD_DATA_DIR).toBe('/tmp/run/skilld')
  })

  it('keeps toolchains and caches at their real paths', () => {
    const env = runEnv(input)
    expect(env.CARGO_HOME).toBe('/opt/cargo')
    expect(env.RUSTUP_HOME).toBe('/home/dev/.rustup')
    expect(env.npm_config_cache).toBe('/home/dev/.npm')
    expect(env.XDG_CACHE_HOME).toBe('/home/dev/.cache')
  })

  it('passes credentials in memory and omits them when there are none', () => {
    expect(runEnv(input).OPENCODE_AUTH_CONTENT).toBe('{"provider":{}}')
    expect(runEnv({ ...input, auth: null }).OPENCODE_AUTH_CONTENT).toBeUndefined()
  })
})
