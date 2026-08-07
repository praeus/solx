/**
 * The `Solx` aggregate — mirrors `solx_surface::Solx` (the trait
 * `types/files/docs/actions/...` accessors).
 *
 * In v1 every entry point is backed by an in-process Neon
 * `Local*` manager. The aggregate mirrors how `solx-cli/src/main.rs`
 * wires the local manager impls together.
 */

import type {
  TypeManager,
  FileStore,
  DocManager,
  ActionManager,
  ConfigService,
  ScriptRunner,
} from '@solx/surface';

export interface Solx {
  readonly types: TypeManager;
  readonly files: FileStore;
  readonly docs: DocManager;
  readonly actions: ActionManager;
  readonly config: ConfigService;
  readonly scripts: ScriptRunner;
  /** Where solx is reading/writing data on disk. */
  readonly appdata: string;
}
