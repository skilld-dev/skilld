/**
 * Render the in-terminal change digest from `/api/cli/changes`.
 *
 * Groups entries by repo, shows the skill name + AI summary if present, and
 * prints a link back to the web view. Format mirrors the email digest minus
 * the HTML wrapper.
 */

import type { ChangeEntry } from '../registry/client.ts'
import { styleText } from 'node:util'
import * as p from '@clack/prompts'

const RELATIVE_FORMATTER = new Intl.RelativeTimeFormat('en', { numeric: 'auto' })

function relative(iso: string, now = Date.now()): string {
  const delta = (new Date(iso).getTime() - now) / 1000
  const minutes = delta / 60
  if (Math.abs(minutes) < 60)
    return RELATIVE_FORMATTER.format(Math.round(minutes), 'minute')
  const hours = minutes / 60
  if (Math.abs(hours) < 24)
    return RELATIVE_FORMATTER.format(Math.round(hours), 'hour')
  return RELATIVE_FORMATTER.format(Math.round(hours / 24), 'day')
}

export function renderDigest(entries: ChangeEntry[]): void {
  if (entries.length === 0) {
    p.log.success('No new updates since last digest.')
    return
  }

  const byRepo = new Map<string, ChangeEntry[]>()
  for (const entry of entries) {
    const list = byRepo.get(entry.repo) ?? []
    list.push(entry)
    byRepo.set(entry.repo, list)
  }

  const lines: string[] = []
  for (const [repo, items] of byRepo) {
    lines.push(styleText('cyan', repo))
    for (const item of items) {
      const when = styleText('gray', relative(item.at))
      lines.push(`  ${styleText('green', '•')} ${item.skill} ${when}`)
      if (item.summary)
        lines.push(`    ${styleText('gray', item.summary)}`)
    }
  }
  lines.push('')
  lines.push(styleText('gray', 'See full activity at https://skilld.dev/me/activity'))

  p.log.message(lines.join('\n'))
}
