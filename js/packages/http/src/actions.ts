/**
 * Pure-fetch `ActionManager` — HTTP proxy for `solx-server`, mirroring
 * `solx-client::RemoteActionManager` (solx-core/solx-client/src/actions.rs).
 * The wire format is camelCase end to end (same as `@solx/surface`'s public
 * types), so request/response bodies pass straight through with no
 * conversion layer.
 */
import type {
  Action,
  ActionExecResult,
  ActionInput,
  ActionManager as ActionManagerIface,
  ActionSearchQuery,
  JsonValue,
  ListOptions,
  Page,
} from '@solx/surface';
import { entityPath, requestJson, toQuery } from './fetch.js';

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
    return requestJson<Action>(this.baseUrl, this.token, 'PUT', entityPath('actions', path, name), {
      body: input,
    });
  }

  async get(path: string, name: string): Promise<Action> {
    return requestJson<Action>(this.baseUrl, this.token, 'GET', entityPath('actions', path, name));
  }

  async delete(path: string, name: string): Promise<void> {
    await requestJson<void>(this.baseUrl, this.token, 'DELETE', entityPath('actions', path, name));
  }

  async list(opts?: ListOptions): Promise<Page<Action>> {
    return requestJson<Page<Action>>(this.baseUrl, this.token, 'GET', '/actions', {
      query: toQuery(opts),
    });
  }

  /**
   * `GET /actions-search` — a top-level route rather than `/actions/search`,
   * for the same reason document search lives at `/search` rather than
   * `/docs/search`; see `HttpDocManager.search`.
   */
  async search(query: ActionSearchQuery): Promise<Page<Action>> {
    return requestJson<Page<Action>>(this.baseUrl, this.token, 'GET', '/actions-search', {
      query: toQuery(query),
    });
  }

  /**
   * `POST` on the action's own URL, with the params as the body — the same
   * path pattern as the CRUD routes, distinguished only by method.
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
