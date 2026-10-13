/**
 * The single choke point every `Tauri*Manager` sends through — the IPC
 * analog of `@solx/http`'s `fetch.ts`. Commands live in the solx Tauri
 * plugin (solx-core/solx-tauri-plugin), so each is invoked as
 * `plugin:solx|<command>`; a rejected call carries the tagged
 * `{kind, message}` `SolxError`, mapped back exactly as an HTTP error body is.
 */
import { mapWireError, SolxError } from '@solx/surface';

/** Raw bytes or a JSON-able argument object — the subset of Tauri's `InvokeArgs` we send. */
export type InvokeArgs = Record<string, unknown> | Uint8Array | ArrayBuffer;

export interface InvokeOptions {
  headers: Record<string, string>;
}

/**
 * Tauri's `invoke` (from `@tauri-apps/api/core`), injected by the host so
 * this package needs no dependency on `@tauri-apps/api`.
 */
export type InvokeFn = (cmd: string, args?: InvokeArgs, options?: InvokeOptions) => Promise<unknown>;

export const PLUGIN = 'solx';

export class TauriTransport {
  constructor(private readonly invoke: InvokeFn) {}

  /**
   * Invoke `plugin:solx|<cmd>`. A unit result (`null` over IPC — deletes,
   * validate) yields `undefined`, matching `@solx/http`'s `204` handling.
   */
  async call<Resp>(cmd: string, args?: InvokeArgs, options?: InvokeOptions): Promise<Resp> {
    let result: unknown;
    try {
      result = await this.invoke(`plugin:${PLUGIN}|${cmd}`, args, options);
    } catch (e) {
      throw toSolxError(e);
    }
    return (result ?? undefined) as Resp;
  }

  /** Like {@link call}, for a command answered with raw bytes (an `ArrayBuffer`). */
  async callBytes(cmd: string, args?: InvokeArgs): Promise<Uint8Array> {
    const result = await this.call<unknown>(cmd, args);
    if (result instanceof ArrayBuffer) return new Uint8Array(result);
    if (result instanceof Uint8Array) return result;
    // Older IPC paths can fall back to a JSON number array.
    if (Array.isArray(result)) return Uint8Array.from(result as number[]);
    throw new SolxError('Other', `solx IPC: expected bytes from ${cmd}, got ${typeof result}`);
  }
}

/**
 * A command's own failure is a serialized `SolxError` object. Anything else
 * — Tauri's own string errors (unknown command, permission denied,
 * argument deserialization) — becomes `SolxError('Other')` with the text.
 */
function toSolxError(e: unknown): SolxError {
  if (e instanceof SolxError) return e;
  if (e && typeof e === 'object' && 'kind' in e) return mapWireError(e);
  const text = e instanceof Error ? e.message : String(e);
  return new SolxError('Other', `solx IPC call failed: ${text}`);
}
