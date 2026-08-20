/**
 * Pure-fetch `ActionManager` — HTTP proxy for `solx-server`, mirroring
 * `solx-client::RemoteActionManager` (solx-core/solx-client/src/actions.rs).
 * Same snake_case<->camelCase conversion pattern as `@solx/actions`'s
 * Neon-backed `native.ts`, but talks `fetch`/JSON directly.
 */
import type {
  Action,
  ActionExecResult,
  ActionInput,
  ActionManager as ActionManagerIface,
  ActionType,
  FileRef,
  JsonValue,
  ListOptions,
  Page,
} from '@solx/surface';
import { entityPath, requestJson, type QueryValue } from './fetch.js';

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

function listOptionsToSnake(opts?: ListOptions): Record<string, QueryValue> {
  if (!opts) return {};
  const out: Record<string, QueryValue> = {};
  if (opts.pathPrefix !== undefined) out['path_prefix'] = opts.pathPrefix;
  if (opts.limit !== undefined) out['limit'] = opts.limit;
  if (opts.offset !== undefined) out['offset'] = opts.offset;
  if (opts.filterField !== undefined) out['filter_field'] = opts.filterField;
  if (opts.filterValue !== undefined) out['filter_value'] = opts.filterValue;
  if (opts.sortBy !== undefined) out['sort_by'] = opts.sortBy;
  if (opts.sortOrder !== undefined) out['sort_order'] = opts.sortOrder;
  if (opts.dateAfter !== undefined) out['date_after'] = opts.dateAfter;
  if (opts.dateBefore !== undefined) out['date_before'] = opts.dateBefore;
  return out;
}

/**
 * HTTP-proxy `ActionManager` talking to a `solx-server`. Public name
 * matches `@solx/actions`'s `ActionManager` and the
 * `solx_surface::managers::ActionManager` trait.
 */
export class HttpActionManager implements ActionManagerIface {
  constructor(
    private readonly baseUrl: string,
    private readonly token: string,
  ) {}

  async save(path: string, name: string, input: ActionInput): Promise<Action> {
    const s = await requestJson<ActionSnake>(
      this.baseUrl,
      this.token,
      'PUT',
      entityPath('actions', path, name),
      { body: actionInputToSnake(input) },
    );
    return actionFromSnake(s);
  }

  async get(path: string, name: string): Promise<Action> {
    const s = await requestJson<ActionSnake>(
      this.baseUrl,
      this.token,
      'GET',
      entityPath('actions', path, name),
    );
    return actionFromSnake(s);
  }

  async delete(path: string, name: string): Promise<void> {
    await requestJson<void>(this.baseUrl, this.token, 'DELETE', entityPath('actions', path, name));
  }

  async list(opts?: ListOptions): Promise<Page<Action>> {
    const s = await requestJson<PageSnake<ActionSnake>>(
      this.baseUrl,
      this.token,
      'GET',
      '/actions',
      { query: listOptionsToSnake(opts) },
    );
    return pageFromSnake(s, actionFromSnake);
  }

  /**
   * `POST` on the action's own URL, with the params as the body — the same
   * path pattern as the CRUD routes, distinguished only by method.
   *
   * `action`/`result`/`success`/`message` are all single-word field names,
   * so no snake_case<->camelCase conversion is needed on the result.
   */
  async exec(path: string, name: string, params: JsonValue = {}): Promise<ActionExecResult> {
    return requestJson<ActionExecResult>(
      this.baseUrl,
      this.token,
      'POST',
      entityPath('actions', path, name),
      { body: params },
    );
  }
}
