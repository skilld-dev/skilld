import type { SearchSnippet } from '../../src/retriv/types'
import { describe, expect, it } from 'vitest'
import { generateSearchGuide, parseAgentFilter, parseFilterPrefix, parseJsonFilter } from '../../src/commands/search'
import { normalizeScores, scoreLabel } from '../../src/core/formatting'

function snippet(overrides: Partial<SearchSnippet> = {}): SearchSnippet {
  return {
    package: 'test-pkg',
    source: 'docs/README.md',
    lineStart: 1,
    lineEnd: 10,
    content: 'test content',
    score: 0.05,
    highlights: [],
    ...overrides,
  }
}

describe('parseFilterPrefix', () => {
  it('returns raw query when no prefix', () => {
    expect(parseFilterPrefix('useFetch options')).toEqual({ query: 'useFetch options' })
  })

  it('parses issues: prefix', () => {
    expect(parseFilterPrefix('issues:memory leak')).toEqual({
      query: 'memory leak',
      filter: { type: 'issue' },
    })
  })

  it('parses issue: prefix (singular)', () => {
    expect(parseFilterPrefix('issue:bug')).toEqual({
      query: 'bug',
      filter: { type: 'issue' },
    })
  })

  it('parses docs: prefix', () => {
    expect(parseFilterPrefix('docs:routing')).toEqual({
      query: 'routing',
      filter: { type: { $in: ['doc', 'docs'] } },
    })
  })

  it('parses releases: prefix', () => {
    expect(parseFilterPrefix('releases:v3')).toEqual({
      query: 'v3',
      filter: { type: 'release' },
    })
  })

  it('is case-insensitive', () => {
    expect(parseFilterPrefix('Issues:bug')).toEqual({
      query: 'bug',
      filter: { type: 'issue' },
    })
  })
})

describe('parseJsonFilter', () => {
  it('parses valid JSON object', () => {
    expect(parseJsonFilter('{"type":"issue"}')).toEqual({ type: 'issue' })
  })

  it('parses filter with operators', () => {
    expect(parseJsonFilter('{"type":{"$in":["doc","issue"]}}')).toEqual({
      type: { $in: ['doc', 'issue'] },
    })
  })

  it('parses $prefix operator', () => {
    expect(parseJsonFilter('{"source":{"$prefix":"docs/api/"}}')).toEqual({
      source: { $prefix: 'docs/api/' },
    })
  })

  it('parses numeric operators', () => {
    expect(parseJsonFilter('{"number":{"$gt":100}}')).toEqual({
      number: { $gt: 100 },
    })
  })

  it('parses $exists operator', () => {
    expect(parseJsonFilter('{"number":{"$exists":true}}')).toEqual({
      number: { $exists: true },
    })
  })

  it('parses multiple fields', () => {
    expect(parseJsonFilter('{"type":"issue","number":{"$lt":50}}')).toEqual({
      type: 'issue',
      number: { $lt: 50 },
    })
  })

  it('returns null for invalid JSON', () => {
    expect(parseJsonFilter('not json')).toBeNull()
  })

  it('returns null for JSON array', () => {
    expect(parseJsonFilter('[1,2,3]')).toBeNull()
  })

  it('returns null for JSON string', () => {
    expect(parseJsonFilter('"hello"')).toBeNull()
  })

  it('returns null for JSON number', () => {
    expect(parseJsonFilter('42')).toBeNull()
  })

  it('returns null for JSON null', () => {
    expect(parseJsonFilter('null')).toBeNull()
  })

  it('returns null for unknown operator', () => {
    expect(parseJsonFilter('{"type":{"$unknown":"value"}}')).toBeNull()
  })

  it('returns null for multi-key operator object', () => {
    expect(parseJsonFilter('{"type":{"$eq":"doc","$ne":"issue"}}')).toBeNull()
  })

  it('returns null for null value', () => {
    expect(parseJsonFilter('{"type":null}')).toBeNull()
  })

  it('returns null for nested non-operator object', () => {
    expect(parseJsonFilter('{"type":{"nested":{"deep":"value"}}}')).toBeNull()
  })

  it('accepts boolean values', () => {
    expect(parseJsonFilter('{"active":true}')).toEqual({ active: true })
  })
})

describe('parseAgentFilter', () => {
  it('parses known agents', () => {
    expect(parseAgentFilter('claude-code, codex')).toEqual({
      _tag: 'Selected',
      agents: ['claude-code', 'codex'],
    })
  })

  it('rejects an empty selection', () => {
    expect(parseAgentFilter(',')).toEqual({ _tag: 'Invalid', values: [] })
  })

  it('rejects inherited object properties', () => {
    expect(parseAgentFilter('toString')).toEqual({ _tag: 'Invalid', values: ['toString'] })
  })
})

describe('generateSearchGuide', () => {
  it('generates generic guide without package', () => {
    const guide = generateSearchGuide()
    expect(guide).toContain('skilld search guide')
    expect(guide).toContain('-p <package>')
    expect(guide).toContain('$prefix')
    expect(guide).toContain('$in')
    expect(guide).toContain('--filter')
    expect(guide).toContain('--limit')
  })

  it('tailors guide to specific package', () => {
    const guide = generateSearchGuide('vue')
    expect(guide).toContain('Search guide for vue')
    expect(guide).toContain('-p vue')
    expect(guide).toContain('e.g. "vue"')
    expect(guide).not.toContain('<package>')
  })

  it('includes all metadata fields', () => {
    const guide = generateSearchGuide()
    expect(guide).toContain('package')
    expect(guide).toContain('source')
    expect(guide).toContain('type')
    expect(guide).toContain('number')
  })

  it('includes all filter operators', () => {
    const guide = generateSearchGuide()
    for (const op of ['$eq', '$ne', '$gt', '$gte', '$lt', '$lte', '$in', '$prefix', '$exists'])
      expect(guide).toContain(op)
  })
})

describe('normalizeScores', () => {
  it('normalizes best result to 100', () => {
    const results = [snippet({ score: 0.08 }), snippet({ score: 0.04 }), snippet({ score: 0.02 })]
    const scores = normalizeScores(results)
    expect(scores.get(results[0]!)).toBe(100)
  })

  it('normalizes relative to best', () => {
    const results = [snippet({ score: 0.10 }), snippet({ score: 0.05 })]
    const scores = normalizeScores(results)
    expect(scores.get(results[1]!)).toBe(50)
  })

  it('handles single result', () => {
    const results = [snippet({ score: 0.03 })]
    const scores = normalizeScores(results)
    expect(scores.get(results[0]!)).toBe(100)
  })

  it('handles zero scores', () => {
    const results = [snippet({ score: 0 })]
    const scores = normalizeScores(results)
    expect(scores.get(results[0]!)).toBe(0)
  })
})

describe('scoreLabel', () => {
  it('returns score with percent suffix', () => {
    expect(scoreLabel(100)).toContain('100%')
    expect(scoreLabel(50)).toContain('50%')
    expect(scoreLabel(20)).toContain('20%')
  })
})
