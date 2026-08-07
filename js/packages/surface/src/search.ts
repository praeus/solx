/**
 * Search hit shape — split out so query.ts can avoid a circular dep
 * (SearchResults references SearchHit).
 */

export interface SearchHit {
  id: string;
  path: string;
  name: string;
  title?: string;
  summary?: string;
  typeRef: string;
  /** Tantivy relevance score (higher = better). */
  score: number;
}
