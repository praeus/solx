/**
 * TS-side wrapper around the `solx-bindings` actions module.
 *
 * Same shape as `@solx/docs`'s `native.ts`. The public class is
 * just `ActionManager` — `open()` (local) and `connect()` (client)
 * both hand back the same class.
 */

import { SolxError } from '@solx/surface';
import type { JsonValue } from '@solx/surface';
import type {
  Action,
  ActionExecResult,
  ActionInput,
  ActionType,
  FileRef,
  ListOptions,
  Page,
} from '@solx/surface';
import { ConfigService } from '@solx/config';
import { DocManager } from '@solx/docs';
import { FileStore } from '@solx/files';
import { TypeManager } from '@solx/types';
import { loadNative } from '@solx/types/loader';
import type { ActionManagerHandle, NativeActionsModule } from './native-types.js';

export type { NativeActionsModule, ActionManagerHandle };

// ----- snake_case DTO mirror (Rust serde shape) -----

interface FileRefSnake {
  name: string;
  rel_path: string;
  content_type?: string;
}

interface ActionSnake {
  id: string;
  path: string;
  name: string;
  caption?: string;
  description?: string;
  capabilities: string[];
  phrases: string[];
  category?: string;
  param_type_ref?: string;
  result_type_ref?: string;
  action_type?: ActionType;
  fn_name?: string;
  bin_name?: string;
  action_config?: JsonValue;
  files: FileRefSnake[];
  trusted: boolean;
  created_at: string;
  updated_at: string;
}

interface ActionInputSnake {
  caption?: string;
  description?: string;
  capabilities?: string[];
  phrases?: string[];
  category?: string;
  param_type_ref?: string;
  result_type_ref?: string;
  action_type?: ActionType;
  fn_name?: string;
  bin_name?: string;
  action_config?: JsonValue;
  files?: FileRefSnake[];
  trusted?: boolean;
}

interface PageSnake<T> {
  items: T[];
  total: number;
  limit: number;
  offset: number;
}

function fileRefFromSnake(s: FileRefSnake): FileRef {
  return { name: s.name, relPath: s.rel_path, contentType: s.content_type };
}

function fileRefToSnake(f: FileRef): FileRefSnake {
  return { name: f.name, rel_path: f.relPath, content_type: f.contentType };
}

function actionFromSnake(s: ActionSnake): Action {
  return {
    id: s.id,
    path: s.path,
    name: s.name,
    caption: s.caption,
    description: s.description,
    capabilities: s.capabilities,
    phrases: s.phrases,
    category: s.category,
    paramTypeRef: s.param_type_ref,
    resultTypeRef: s.result_type_ref,
    actionType: s.action_type,
    fnName: s.fn_name,
    binName: s.bin_name,
    actionConfig: s.action_config as Action['actionConfig'],
    files: s.files.map(fileRefFromSnake),
    trusted: s.trusted,
    createdAt: s.created_at,
    updatedAt: s.updated_at,
  };
}

function actionInputToSnake(i: ActionInput): ActionInputSnake {
  const out: ActionInputSnake = {};
  if (i.caption !== undefined) out.caption = i.caption;
  if (i.description !== undefined) out.description = i.description;
  if (i.capabilities !== undefined) out.capabilities = i.capabilities;
  if (i.phrases !== undefined) out.phrases = i.phrases;
  if (i.category !== undefined) out.category = i.category;
  if (i.paramTypeRef !== undefined) out.param_type_ref = i.paramTypeRef;
  if (i.resultTypeRef !== undefined) out.result_type_ref = i.resultTypeRef;
  if (i.actionType !== undefined) out.action_type = i.actionType;
  if (i.fnName !== undefined) out.fn_name = i.fnName;
  if (i.binName !== undefined) out.bin_name = i.binName;
  if (i.actionConfig !== undefined) out.action_config = i.actionConfig;
  if (i.files !== undefined) out.files = i.files.map(fileRefToSnake);
  if (i.trusted !== undefined) out.trusted = i.trusted;
  return out;
}

function pageFromSnake<T, S>(s: PageSnake<S>, fromSnake: (x: S) => T): Page<T> {
  return { items: s.items.map(fromSnake), total: s.total, limit: s.limit, offset: s.offset };
}

function listOptionsToJson(opts?: ListOptions): string {
  if (!opts) return '{}';
  const out: Record<string, JsonValue> = {};
  if (opts.pathPrefix !== undefined) out['path_prefix'] = opts.pathPrefix;
  if (opts.limit !== undefined) out['limit'] = opts.limit;
  if (opts.offset !== undefined) out['offset'] = opts.offset;
  if (opts.filterField !== undefined) out['filter_field'] = opts.filterField;
  if (opts.filterValue !== undefined) out['filter_value'] = opts.filterValue;
  if (opts.sortBy !== undefined) out['sort_by'] = opts.sortBy;
  if (opts.sortOrder !== undefined) out['sort_order'] = opts.sortOrder;
  if (opts.dateAfter !== undefined) out['date_after'] = opts.dateAfter;
  if (opts.dateBefore !== undefined) out['date_before'] = opts.dateBefore;
  return JSON.stringify(out);
}

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

  async post(path: string, name: string, input: ActionInput): Promise<Action> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeActionsModule;
      const json = await native.actionsPost(
        this.handle,
        path,
        name,
        JSON.stringify(actionInputToSnake(input)),
      );
      return actionFromSnake(JSON.parse(json) as ActionSnake);
    });
  }

  async get(path: string, name: string): Promise<Action> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeActionsModule;
      const json = await native.actionsGet(this.handle, path, name);
      return actionFromSnake(JSON.parse(json) as ActionSnake);
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
      const json = await native.actionsList(this.handle, listOptionsToJson(opts));
      return pageFromSnake(JSON.parse(json) as PageSnake<ActionSnake>, actionFromSnake);
    });
  }

  async exec(path: string, name: string, params: JsonValue = {}): Promise<ActionExecResult> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeActionsModule;
      const json = await native.actionsExec(this.handle, path, name, JSON.stringify(params));
      // `action`/`result`/`success`/`message` are all single-word
      // field names — no snake_case↔camelCase conversion needed.
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
