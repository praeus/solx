/**
 * Re-export the public API of `@solx/surface` plus the
 * `DocManager` class. Consumers can
 * `import { DocManager, Document } from 'solx/docs'`.
 *
 * @packageDocumentation
 */

export { DocManager } from './native.js';
export type { NativeDocsModule, DocManagerHandle } from './native-types.js';
export type {
  Document,
  DocumentInput,
  DocLink,
  FileRef,
  ListOptions,
  Page,
  SearchHit,
  SearchQuery,
  SearchResults,
  JsonValue,
  SolxErrorKind,
} from '@solx/surface';
export { SolxError, LinkKind } from '@solx/surface';
