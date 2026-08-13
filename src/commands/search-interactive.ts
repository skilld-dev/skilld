import type { AgentType } from '../agent/index.ts'
import type { SearchFilter, SearchSnippet } from '../retriv/index.ts'
import { styleText } from 'node:util'
import { createLogUpdate } from 'log-update'
import { formatCompactSnippet, highlightTerms, normalizeScores, sanitizeMarkdown, scoreLabel } from '../core/index.ts'
import { closePool, openPool, SearchDepsUnavailableError, searchPooled } from '../retriv/index.ts'
import { findPackageDbs, getPackageVersions, listLockPackages, parseFilterPrefix } from './search-helpers.ts'

const FILTER_CYCLE = [undefined, 'docs', 'issues', 'releases'] as const
type FilterLabel = typeof FILTER_CYCLE[number]

function filterToSearchFilter(label: FilterLabel): SearchFilter | undefined {
  if (!label)
    return undefined
  if (label === 'issues')
    return { type: 'issue' }
  if (label === 'releases')
    return { type: 'release' }
  return { type: { $in: ['doc', 'docs'] } }
}

const SPINNER_FRAMES = ['◐', '◓', '◑', '◒']

export async function interactiveSearch(packageFilter?: string, agentTypes?: AgentType[]): Promise<void> {
  const dbs = findPackageDbs(packageFilter, agentTypes)
  const versions = getPackageVersions(process.cwd(), agentTypes)
  if (dbs.length === 0) {
    let msg: string
    if (packageFilter) {
      const available = listLockPackages(process.cwd(), agentTypes)
      msg = available.length > 0
        ? `No docs indexed for "${packageFilter}". Available: ${available.join(', ')}`
        : `No docs indexed for "${packageFilter}". Run \`skilld add ${packageFilter}\` first.`
    }
    else {
      msg = 'No docs indexed yet. Run `skilld add <package>` first.'
    }
    process.stderr.write(`${styleText('yellow', msg)}\n`)
    return
  }

  const logUpdate = createLogUpdate(process.stderr, { showCursor: true })
  let pool: Awaited<ReturnType<typeof openPool>>
  try {
    pool = await openPool(dbs)
  }
  catch (err) {
    if (err instanceof SearchDepsUnavailableError) {
      process.stderr.write(`${styleText('red', 'Search requires native dependencies (sqlite-vec) that are not installed.\nInstall skilld globally or in a project to use search: npm i -g skilld')}\n`)
      return
    }
    throw err
  }

  // State
  let query = ''
  let results: SearchSnippet[] = []
  let selectedIndex = 0
  let isSearching = false
  let searchId = 0
  let filterIndex = 0
  let error = ''
  let elapsed = 0
  let spinFrame = 0
  let debounceTimer: ReturnType<typeof setTimeout> | null = null

  const cols = process.stdout.columns || 80
  const maxResults = 7
  const titleLabel = packageFilter ? `Search ${packageFilter} docs` : 'Search docs'

  function getFilterLabel(): string {
    const f = FILTER_CYCLE[filterIndex]
    if (!f)
      return ''
    return styleText('cyan', `${f}:`)
  }

  function render() {
    const lines: string[] = []

    // Title
    lines.push('')
    lines.push(`  ${styleText('bold', titleLabel)}`)
    lines.push('')

    // Input line
    const filterPrefix = getFilterLabel()
    const prefix = filterPrefix ? `${filterPrefix}` : ''
    lines.push(`  ${styleText('cyan', '❯')} ${prefix}${query}${styleText('inverse', ' ')}`)

    // Separator / spinner
    if (isSearching) {
      const frame = SPINNER_FRAMES[spinFrame % SPINNER_FRAMES.length]!
      lines.push(`  ${styleText('cyan', frame)} ${styleText('gray', 'Searching…')}`)
    }
    else {
      lines.push(`  ${styleText('gray', '─'.repeat(Math.min(cols - 4, 40)))}`)
    }

    // Results or empty state
    if (error) {
      lines.push('')
      lines.push(`  ${styleText('red', error)}`)
    }
    else if (query.length === 0) {
      lines.push('')
      lines.push(`  ${styleText('gray', 'Type to search…')}`)
    }
    else if (query.length < 2 && !isSearching) {
      lines.push('')
      lines.push(`  ${styleText('gray', 'Keep typing…')}`)
    }
    else if (results.length === 0 && !isSearching) {
      lines.push('')
      lines.push(`  ${styleText('gray', 'No results')}`)
    }
    else {
      lines.push('')
      const shown = results.slice(0, maxResults)
      const scores = normalizeScores(results)
      for (let i = 0; i < shown.length; i++) {
        const r = shown[i]!
        const selected = i === selectedIndex
        const bullet = selected ? styleText('cyan', '●') : styleText('gray', '○')
        const sc = scoreLabel(scores.get(r) ?? 0)
        const { title, path, preview } = formatCompactSnippet(r, cols)
        const highlighted = highlightTerms(preview, r.highlights)

        const ver = versions.get(r.package)
        const pkgLabel = ver ? `${r.package}@${ver}` : r.package

        if (selected) {
          lines.push(`  ${bullet} ${styleText('bold', pkgLabel)} ${sc}  ${styleText('cyan', title)}`)
          lines.push(`    ${styleText('gray', path)}`)
          lines.push(`    ${highlighted}`)
        }
        else {
          lines.push(`  ${bullet} ${styleText('gray', pkgLabel)} ${sc}  ${styleText('gray', title)}`)
        }
      }
    }

    // Footer
    lines.push('')
    const parts: string[] = []
    if (results.length > 0)
      parts.push(`${results.length} results`)
    if (elapsed > 0 && !isSearching)
      parts.push(`${elapsed.toFixed(2)}s`)
    const footer = parts.length > 0 ? `${parts.join(' · ')}    ` : ''
    lines.push(`  ${styleText('gray', `${footer}↑↓ navigate  ↵ select  tab filter  esc quit`)}`)
    lines.push('')

    logUpdate(lines.join('\n'))
  }

  async function doSearch() {
    const id = ++searchId
    const fullQuery = query.trim()
    if (fullQuery.length < 2) {
      results = []
      isSearching = false
      render()
      return
    }

    isSearching = true
    error = ''
    render()

    // Spin animation
    const spinInterval = setInterval(() => {
      spinFrame++
      if (isSearching)
        render()
    }, 80)

    const { query: parsed, filter: parsedFilter } = parseFilterPrefix(fullQuery)
    const filter = parsedFilter || filterToSearchFilter(FILTER_CYCLE[filterIndex])
    const start = performance.now()

    const res = await searchPooled(parsed, pool, { limit: maxResults, filter }).catch((e) => {
      if (id === searchId)
        error = e instanceof Error ? e.message : String(e)
      return [] as SearchSnippet[]
    })

    clearInterval(spinInterval)

    // Discard stale results
    if (id !== searchId)
      return

    results = res
    elapsed = (performance.now() - start) / 1000
    selectedIndex = 0
    isSearching = false
    render()
  }

  function scheduleSearch() {
    if (debounceTimer)
      clearTimeout(debounceTimer)
    debounceTimer = setTimeout(doSearch, 100)
  }

  // Show initial state
  render()

  // Raw stdin for keystroke handling
  const { stdin } = process
  if (stdin.isTTY)
    stdin.setRawMode(true)
  stdin.resume()
  stdin.setEncoding('utf-8')

  return new Promise<void>((resolve) => {
    function cleanup() {
      if (debounceTimer)
        clearTimeout(debounceTimer)
      if (stdin.isTTY)
        stdin.setRawMode(false)
      stdin.removeListener('data', onData)
      stdin.pause()
      closePool(pool)
    }

    function exit() {
      cleanup()
      logUpdate.done()
      resolve()
    }

    function selectResult() {
      if (results.length === 0 || selectedIndex >= results.length)
        return
      const r = results[selectedIndex]!
      cleanup()
      logUpdate.done()

      // Print full result
      const refPath = `.claude/skills/${r.package}/.skilld/${r.source}`
      const lineRange = r.lineStart === r.lineEnd ? `L${r.lineStart}` : `L${r.lineStart}-${r.lineEnd}`
      const highlighted = highlightTerms(sanitizeMarkdown(r.content), r.highlights)
      const rVer = versions.get(r.package)
      const rLabel = rVer ? `${r.package}@${rVer}` : r.package
      const rScores = normalizeScores(results)
      const out = [
        '',
        `  ${styleText('bold', rLabel)} ${scoreLabel(rScores.get(r) ?? 0)}`,
        `  ${styleText('gray', `${refPath}:${lineRange}`)}`,
        '',
        `  ${highlighted.replace(/\n/g, '\n  ')}`,
        '',
      ].join('\n')
      process.stdout.write(`${out}\n`)
      resolve()
    }

    function onData(data: string) {
      // Ctrl+C
      if (data === '\x03') {
        exit()
        return
      }

      // Escape
      if (data === '\x1B' || data === '\x1B\x1B') {
        exit()
        return
      }

      // Enter
      if (data === '\r' || data === '\n') {
        selectResult()
        return
      }

      // Tab — cycle filter
      if (data === '\t') {
        filterIndex = (filterIndex + 1) % FILTER_CYCLE.length
        if (query.length >= 2)
          scheduleSearch()
        render()
        return
      }

      // Backspace
      if (data === '\x7F' || data === '\b') {
        if (query.length > 0) {
          query = query.slice(0, -1)
          scheduleSearch()
          render()
        }
        return
      }

      // Arrow keys (escape sequences)
      if (data === '\x1B[A' || data === '\x1BOA') {
        // Up
        if (selectedIndex > 0) {
          selectedIndex--
          render()
        }
        return
      }
      if (data === '\x1B[B' || data === '\x1BOB') {
        // Down
        if (selectedIndex < results.length - 1) {
          selectedIndex++
          render()
        }
        return
      }

      // Ignore other escape sequences
      if (data.startsWith('\x1B'))
        return

      // Printable characters
      query += data
      scheduleSearch()
      render()
    }

    stdin.on('data', onData)
  })
}
