/**
 * @solx/sdk — the umbrella `solx` package.
 *
 * This is what end users install. Everything is re-exported under
 * one root for convenience, and under subpath exports for
 * tree-shaking (`solx/types`, `solx/config`, etc.).
 *
 * @packageDocumentation
 */

export { createSolx } from './create.js';
export type { CreateSolxOptions } from './create.js';
export type { Solx } from './solx.js';

// Re-export @solx/surface under the umbrella too.
export {
  SolxError,
  Path,
  Name,
  Ref,
  JsonValue,
  LinkKind,
  ActionType,
  SortOrder,
} from '@solx/surface';
export type {
  SolxErrorKind,
  JsonObject,
  TypeEntity,
  TypeInput,
  Document,
  DocumentInput,
  DocLink,
  FileRef,
  Action,
  ActionInput,
  ActionExecResult,
  SearchHit,
  SearchQuery,
  SearchResults,
  ListOptions,
  Page,
  // Manager interfaces. Note: `TypeManager`, `FileStore`,
  // `ConfigService`, `DocManager`, `ActionManager`, and
  // `ScriptRunner` are also re-exported as **classes** below
  // (real bindings for all six). TypeScript unifies class-as-
  // type and class-as-value, so the surface interfaces
  // themselves are not re-exported here.
  SolxConfig,
  InstalledPackage,
} from '@solx/surface';

// Re-export @solx/types under the umbrella.
export { TypeManager } from '@solx/types';
export type { NativeTypesModule, TypeManagerHandle } from '@solx/types';

// Re-export @solx/files under the umbrella.
export { FileStore, FilePath } from '@solx/files';
export type { NativeFilesModule, FileStoreHandle } from '@solx/files';

// Re-export @solx/config under the umbrella.
export { ConfigService } from '@solx/config';
export type { NativeConfigModule, ConfigServiceHandle } from '@solx/config';

// Re-export @solx/docs under the umbrella.
export { DocManager } from '@solx/docs';
export type { NativeDocsModule, DocManagerHandle } from '@solx/docs';

// Re-export @solx/actions under the umbrella.
export { ActionManager } from '@solx/actions';
export type { NativeActionsModule, ActionManagerHandle } from '@solx/actions';

// Re-export @solx/scripts under the umbrella.
export { ScriptRunner } from '@solx/scripts';
export type { NativeScriptsModule } from '@solx/scripts';
