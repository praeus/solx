/**
 * TS-side wrapper around the `solx-bindings` actions module.
 *
 * Same shape as `@solx/docs`'s `native.ts`. The public class is
 * just `ActionManager` — `open()` (local) and `connect()` (client)
 * both hand back the same class. The Neon boundary marshals the same
 * `solx_surface::entities` structs as `solx-server`'s HTTP responses, so the
 * JSON crossing it is camelCase and needs no conversion on this side.
 */

import { SolxError } from '@solx/surface';
import type { JsonValue } from '@solx/surface';
import type { Action, ActionExecResult, ActionInput, ListOptions, Page } from '@solx/surface';
import { ConfigService } from '@solx/config';
import { DocManager } from '@solx/docs';
import { FileStore } from '@solx/files';
import { TypeManager } from '@solx/types';
import { loadNative } from '@solx/types/loader';
import type { ActionManagerHandle, NativeActionsModule } from './native-types.js';

export type { NativeActionsModule, ActionManagerHandle };

async function withSolxError<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn();
  } catch (e) {
    throw SolxError.fromMessage(e);
  }
}

// ----- ActionManager -----

export class ActionManager {
  private constructor(private readonly handle: ActionManagerHandle) {}

  /**
   * Open a local action store. Mirrors
   * `solx-manager::App::wire_local`'s actions step: needs `config`
   * (env-mapping allowlist for the `get_env` built-in) plus the
   * `types`/`docs`/`files` managers actions dispatch into.
   */
  static async open(
    dbPath: string,
    config: ConfigService,
    types: TypeManager,
    docs: DocManager,
    files: FileStore,
  ): Promise<ActionManager> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeActionsModule;
      const handle = native.actionsOpen(
        dbPath,
        ConfigService.handleOf(config),
        TypeManager.handleOf(types),
        DocManager.handleOf(docs),
        FileStore.handleOf(files),
      );
      return new ActionManager(handle);
    });
  }

  /** Connect to a remote `solx-server` instead of opening a local database. */
  static async connect(serverUrl: string, token: string): Promise<ActionManager> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeActionsModule;
      const handle = native.actionsConnect(serverUrl, token);
      return new ActionManager(handle);
    });
  }

  async save(path: string, name: string, input: ActionInput): Promise<Action> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeActionsModule;
      const json = await native.actionsSave(this.handle, path, name, JSON.stringify(input));
      return JSON.parse(json) as Action;
    });
  }

  async get(path: string, name: string): Promise<Action> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeActionsModule;
      const json = await native.actionsGet(this.handle, path, name);
      return JSON.parse(json) as Action;
    });
  }

  async delete(path: string, name: string): Promise<void> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeActionsModule;
      await native.actionsDelete(this.handle, path, name);
    });
  }

  async list(opts?: ListOptions): Promise<Page<Action>> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeActionsModule;
      const json = await native.actionsList(this.handle, JSON.stringify(opts ?? {}));
      return JSON.parse(json) as Page<Action>;
    });
  }

  async exec(path: string, name: string, params: JsonValue = {}): Promise<ActionExecResult> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeActionsModule;
      const json = await native.actionsExec(this.handle, path, name, JSON.stringify(params));
      return JSON.parse(json) as ActionExecResult;
    });
  }

  /**
   * @internal Escape hatch for dependent managers (`ScriptRunner`).
   * Not part of the public API — do not call from application code.
   */
  static handleOf(mgr: ActionManager): ActionManagerHandle {
    return mgr.handle;
  }
}
