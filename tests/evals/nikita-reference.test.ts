import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { loadCase } from '../../scripts/eval-opencode/case.ts'
import { gradeRun, runScore } from '../../scripts/eval-opencode/grade.ts'
import { parseTranscript } from '../../scripts/eval-opencode/transcript.ts'

const commit = 'f8b8a1828cd53e4aedaa39c3e2abb01e71660bc0'
const selector = 'heyimjames/nikita-bier-consumer-apps/nikita-bier-consumer-apps'

function grade(referenceCommit = commit, skillCommit = commit) {
  const loaded = loadCase(fileURLToPath(new URL('../../evals/skilld-nikita-reference', import.meta.url)))
  if (loaded._tag === 'Err')
    throw new Error(loaded.message)
  const tool = (command: string, output: string) => JSON.stringify({
    type: 'tool_use',
    part: { tool: 'bash', state: { input: { command }, output, status: 'completed', metadata: { exit: 0 } } },
  })
  const transcript = parseTranscript([
    tool(`skilld run 'github:heyimjames/nikita-bier-consumer-apps/.#commit:${skillCommit}' --json`,
      `{"readArgv":["--revision","${skillCommit}"]}${'long supporting content'.repeat(300)}`),
    tool(`skilld run ${selector} --revision ${referenceCommit} --file=onboarding.md --json`, 'reference'),
    JSON.stringify({ type: 'text', part: { text: `Community codes, QR codes, deep links, phone numbers. ${commit}` } }),
  ].join('\n'))
  return gradeRun(loaded.value.graders, { transcript, readProjectFile: () => null, verdicts: new Map(), arm: 'without' })
}

describe('Nikita reference eval', () => {
  it('accepts the pinned source workflow when tool output exceeds the trace limit', () => {
    expect(runScore(grade())).toBe(1)
  })

  it('rejects a reference from another commit', () => {
    expect(grade('0'.repeat(40)).find(result => result.name === 'pinned-reference')?.outcome)
      .toMatchObject({ _tag: 'Scored', score: 0 })
  })

  it('rejects a Skill loaded from another commit', () => {
    expect(grade(commit, '0'.repeat(40)).find(result => result.name === 'loaded-skill')?.outcome)
      .toMatchObject({ _tag: 'Scored', score: 0 })
  })
})
