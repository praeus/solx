/**
 * Exercises the IPC surface `@solx/tauri` speaks, against a fake `invoke`.
 * The assertions are about the *wire*: which `plugin:solx|<command>` each
 * manager call produces and with which argument names (they must match the
 * Rust command parameters in solx-tauri-plugin's `commands.rs`, camelCased
 * by Tauri), how raw file bytes are sent and received, and how rejections
 * map to `SolxError`.
 */
import { beforeEach, describe, expect, test } from 'vitest';
import { SolxError } from '@solx/surface';
import { connectTauri, type InvokeArgs, type InvokeOptions, type TauriSolx } from '../src/index.js';

interface Call {
  cmd: string;
  args?: InvokeArgs;
  options?: InvokeOptions;
}

let calls: Call[];
let reply: (call: Call) => unknown;
let solx: TauriSolx;

beforeEach(() => {
  calls = [];
  reply = () => null;
  solx = connectTauri(async (cmd, args, options) => {
    const call = { cmd, args, options };
    calls.push(call);
    return reply(call);
  });
});

function last(): Call {
  return calls[calls.length - 1]!;
}

describe('entity commands', () => {
  test('save/get/delete pass path, name and input by name', async () => {
    reply = () => ({ path: '/a', name: 'n' });
    await solx.docs.save('/a', 'n', { contents: {} } as never);
    expect(last()).toEqual({
      cmd: 'plugin:solx|docs_save',
      args: { path: '/a', name: 'n', input: { contents: {} } },
      options: undefined,
    });

    await solx.types.get('/t', 'T');
    expect(last().cmd).toBe('plugin:solx|types_get');
    expect(last().args).toEqual({ path: '/t', name: 'T' });

    await expect(solx.actions.delete('/x', 'y')).resolves.toBeUndefined();
    expect(last().cmd).toBe('plugin:solx|actions_delete');
  });

  test('list, paths and search pass their options object', async () => {
    reply = () => ({ items: [], total: 0, limit: 10, offset: 0 });
    await solx.docs.list({ pathPrefix: '/a', limit: 10 });
    expect(last()).toMatchObject({ cmd: 'plugin:solx|docs_list', args: { opts: { pathPrefix: '/a', limit: 10 } } });

    await solx.actions.paths();
    expect(last()).toMatchObject({ cmd: 'plugin:solx|actions_paths', args: { opts: undefined } });

    await solx.docs.search({ q: 'hello' });
    expect(last()).toMatchObject({ cmd: 'plugin:solx|docs_search', args: { query: { q: 'hello' } } });

    await solx.actions.search({ q: 'x', excludeHidden: true });
    expect(last()).toMatchObject({ cmd: 'plugin:solx|actions_search', args: { query: { q: 'x', excludeHidden: true } } });
  });

  test('types resolve/validate use typeRef', async () => {
    await solx.types.resolve('/t/T');
    expect(last()).toMatchObject({ cmd: 'plugin:solx|types_resolve', args: { typeRef: '/t/T' } });

    await expect(solx.types.validate({ a: 1 }, '/t/T')).resolves.toBeUndefined();
    expect(last()).toMatchObject({ cmd: 'plugin:solx|types_validate', args: { value: { a: 1 }, typeRef: '/t/T' } });
  });

  test('exec defaults params to {}', async () => {
    reply = () => ({ output: 1 });
    await expect(solx.actions.exec('/a', 'run')).resolves.toEqual({ output: 1 });
    expect(last()).toMatchObject({ cmd: 'plugin:solx|actions_exec', args: { path: '/a', name: 'run', params: {} } });
  });
});

describe('files', () => {
  test('put sends raw bytes with the encoded path in a header', async () => {
    reply = () => 'img/a b.png';
    const bytes = new Uint8Array([1, 2, 3]);
    await expect(solx.files.put('img/a b.png', bytes)).resolves.toBe('img/a b.png');
    expect(last().cmd).toBe('plugin:solx|files_put');
    expect(last().args).toBe(bytes);
    expect(last().options).toEqual({ headers: { 'x-solx-path': 'img%2Fa%20b.png' } });
  });

  test('get accepts an ArrayBuffer (or a number array) as raw bytes', async () => {
    reply = () => new Uint8Array([9, 8]).buffer;
    await expect(solx.files.get('x.bin')).resolves.toEqual(new Uint8Array([9, 8]));
    expect(last()).toMatchObject({ cmd: 'plugin:solx|files_get', args: { relPath: 'x.bin' } });

    reply = () => [7];
    await expect(solx.files.get('y.bin')).resolves.toEqual(new Uint8Array([7]));
  });

  test('list passes the prefix and returns the bare path list', async () => {
    reply = () => ['a/1', 'a/2'];
    await expect(solx.files.list('a/')).resolves.toEqual(['a/1', 'a/2']);
    expect(last()).toMatchObject({ cmd: 'plugin:solx|files_list', args: { prefix: 'a/' } });
  });
});

describe('host', () => {
  test('info and connect', async () => {
    reply = () => ({ mode: 'local', client: false, version: '0.1.0' });
    await expect(solx.host.info()).resolves.toMatchObject({ mode: 'local' });
    expect(last().cmd).toBe('plugin:solx|info');

    reply = () => null;
    await solx.host.connect('http://h:8766', 'tok');
    expect(last()).toMatchObject({ cmd: 'plugin:solx|connect', args: { serverUrl: 'http://h:8766', token: 'tok' } });
  });
});

describe('config', () => {
  test('get and update send snake_case changes, null removing a key', async () => {
    reply = () => ({ raw: { server_port: 9000 }, effective: {}, path: 'p', appdata: 'a', restartKeys: [] });
    await expect(solx.config.get()).resolves.toMatchObject({ raw: { server_port: 9000 } });
    expect(last()).toEqual({ cmd: 'plugin:solx|config_get', args: undefined, options: undefined });

    reply = () => ({ config: {}, changedKeys: ['server_port'], restartRequired: ['server_port'] });
    await solx.config.update({ server_port: 9001, console_ttl_days: null });
    expect(last()).toMatchObject({
      cmd: 'plugin:solx|config_update',
      args: { changes: { server_port: 9001, console_ttl_days: null } },
    });
  });

  test('a rejected write surfaces as a Validation error', async () => {
    reply = () => {
      throw { kind: 'validation', message: 'server_host "x" is not an IP address' };
    };
    const err = await solx.config.update({ server_host: 'x' }).catch((e: unknown) => e);
    expect((err as SolxError).kind).toBe('Validation');
  });
});

describe('errors', () => {
  test('a tagged SolxError rejection maps to the matching kind', async () => {
    reply = () => {
      throw { kind: 'not_found', message: 'doc /a/n' };
    };
    const err = await solx.docs.get('/a', 'n').catch((e: unknown) => e);
    expect(err).toBeInstanceOf(SolxError);
    expect((err as SolxError).kind).toBe('NotFound');
    expect((err as SolxError).detail).toBe('doc /a/n');
  });

  test("Tauri's own string errors become Other", async () => {
    reply = () => {
      throw 'Command docs_get not allowed by ACL';
    };
    const err = await solx.docs.get('/a', 'n').catch((e: unknown) => e);
    expect(err).toBeInstanceOf(SolxError);
    expect((err as SolxError).kind).toBe('Other');
    expect((err as SolxError).detail).toContain('not allowed by ACL');
  });
});
