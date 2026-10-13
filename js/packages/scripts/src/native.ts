/**
 * TS-side wrapper around the `solx-bindings` scripts module.
 *
 * There is no `ScriptRunner` trait in solx-core — the Rust binding
 * runs a small `CommandRunner` over `Arc<dyn ActionManager>` that
 * supports exactly two stage verbs: `exec <path/name> [--json
 * '<params>']` and `json <literal>`. See `@solx/surface`'s
 * `ScriptRunner` interface doc for the full rationale.
 *
 * `ScriptRunner.for(actions)` binds a runner to an already-open
 * `ActionManager` — there is no separate `open`/`connect`, since
 * scripts just dispatch through whatever actions handle they're
 * given (local or remote, transparently).
 */

import { SolxError } from '@solx/surface';
import type { JsonValue } from '@solx/surface';
import { ActionManager } from '@solx/actions';
import { loadNative } from '@solx/types/loader';
import type { NativeScriptsModule } from './native-types.js';

export type { NativeScriptsModule };

async function withSolxError<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn();
  } catch (e) {
    throw SolxError.fromMessage(e);
  }
}

export class ScriptRunner {
  private constructor(private readonly actionsHandle: unknown) {}

  /** Bind a script runner to an already-open `ActionManager`. */
  static for(actions: ActionManager): ScriptRunner {
    return new ScriptRunner(ActionManager.handleOf(actions));
  }

  async run(source: string, params?: JsonValue): Promise<JsonValue> {
    return withSolxError(async () => {
      const native = loadNative() as unknown as NativeScriptsModule;
      const json = await native.scriptsRun(
        this.actionsHandle,
        source,
        params !== undefined ? JSON.stringify(params) : undefined,
      );
      return JSON.parse(json) as JsonValue;
    });
  }
}
