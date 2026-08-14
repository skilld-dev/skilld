export interface ChunkEntity {
  name: string
  type: string
  signature?: string
  isPartial?: boolean
}

export interface Document {
  id: string
  content: string
  metadata?: Record<string, any>
}

export interface ChunkingOptions {
  chunkSize?: number
  chunkOverlap?: number
}

export type IndexPhase = 'chunking' | 'embedding' | 'storing'

export interface IndexProgress {
  phase: IndexPhase
  current: number
  total: number
}

export interface IndexConfig {
  dbPath: string
  model?: string
  chunking?: ChunkingOptions
  /** Progress callback forwarded from retriv */
  onProgress?: (progress: IndexProgress) => void
}

export interface SearchResult {
  id: string
  content: string
  score: number
  metadata: Record<string, any>
  highlights: string[]
  /** Line range from chunk [start, end] */
  lineRange?: [number, number]
  entities?: ChunkEntity[]
  scope?: ChunkEntity[]
}

export type FilterOperator
  = | { $eq: string | number | boolean }
    | { $ne: string | number | boolean }
    | { $gt: number }
    | { $gte: number }
    | { $lt: number }
    | { $lte: number }
    | { $in: (string | number)[] }
    | { $prefix: string }
    | { $exists: boolean }

export type FilterValue = string | number | boolean | FilterOperator
export type SearchFilter = Record<string, FilterValue>

export interface SearchOptions {
  /** Max results */
  limit?: number
  /** Filter by metadata fields */
  filter?: SearchFilter
}

export interface SearchSnippet {
  /** Package name and version */
  package: string
  /** Source file path */
  source: string
  /** Project-relative directory containing the referenced source tree */
  referenceRoot?: string
  /** Start line number */
  lineStart: number
  /** End line number */
  lineEnd: number
  /** Snippet content (5 lines around best match) */
  content: string
  /** Relevance score */
  score: number
  /** Matched query terms, ordered by BM25 score */
  highlights: string[]
  /** Entities defined in this chunk */
  entities?: ChunkEntity[]
  /** Containing scope chain */
  scope?: ChunkEntity[]
}
