/**
 * Re-export the public API of `@solx/surface` plus the
 * `ActionManager` class. Consumers can
 * `import { ActionManager, Action } from 'solx/actions'`.
 *
 * @packageDocumentation
 */

export { ActionManager } from './native.js';
export type { NativeActionsModule, ActionManagerHandle } from './native-types.js';
export type {
  Action,
  ActionExecResult,
  ActionInput,
  FileRef,
  ListOptions,
  Page,
  JsonValue,
  SolxErrorKind,
} from '@solx/surface';
export { SolxError, ActionType } from '@solx/surface';
