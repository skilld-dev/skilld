/**
 * Classify a cached doc path into the right metadata type.
 * Pure utility — no I/O.
 */
export function classifyCachedDoc(path: string): { type: string, number?: number } {
  const issueMatch = path.match(/^issues\/issue-(\d+)\.md$/)
  if (issueMatch)
    return { type: 'issue', number: Number(issueMatch[1]) }
  const discussionMatch = path.match(/^discussions\/discussion-(\d+)\.md$/)
  if (discussionMatch)
    return { type: 'discussion', number: Number(discussionMatch[1]) }
  if (path.startsWith('releases/'))
    return { type: 'release' }
  return { type: 'doc' }
}
