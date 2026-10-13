/**
 * IPC `ActionManager` — `plugin:solx|actions_*`, mirroring `@solx/http`'s
 * `HttpActionManager` method-for-method.
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
  PathFacet,
} from '@solx/surface';
import type { TauriTransport } from './invoke.js';

export class TauriActionManager implements ActionManagerIface {
  constructor(private readonly t: TauriTransport) {}

  save(path: string, name: string, input: ActionInput): Promise<Action> {
    return this.t.call('actions_save', { path, name, input });
  }

  get(path: string, name: string): Promise<Action> {
    return this.t.call('actions_get', { path, name });
  }

  async delete(path: string, name: string): Promise<void> {
    await this.t.call('actions_delete', { path, name });
  }

  list(opts?: ListOptions): Promise<Page<Action>> {
    return this.t.call('actions_list', { opts });
  }

  paths(opts?: ListOptions): Promise<Page<PathFacet>> {
    return this.t.call('actions_paths', { opts });
  }

  search(query: ActionSearchQuery): Promise<Page<Action>> {
    return this.t.call('actions_search', { query });
  }

  exec(path: string, name: string, params: JsonValue = {}): Promise<ActionExecResult> {
    return this.t.call('actions_exec', { path, name, params });
  }
}
