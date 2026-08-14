/**
 * Round-trip test for `@solx/actions` end-to-end (local mode).
 *
 * Skips if the native binary is not built. Run
 * `cargo build -p solx-bindings` first, set `SOLX_NATIVE_BIN` to the
 * resulting `solx.node`, then run `bun run test`.
 */

import { afterAll, beforeAll, describe, expect, test } from 'vitest';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { SolxError } from '@solx/surface';
import { ConfigService } from '@solx/config';
import { TypeManager } from '@solx/types';
import { FileStore } from '@solx/files';
import { DocManager } from '@solx/docs';
import { ActionManager } from '../src/index.js';

let nativePath: string | undefined;
const nativeEnv = process.env['SOLX_NATIVE_BIN'];
if (nativeEnv && existsSync(nativeEnv)) {
  nativePath = nativeEnv;
  process.env['SOLX_NODE_PATH'] = nativePath;
}

const describeIfNative = nativePath ? describe : describe.skip;

describeIfNative('ActionManager (round-trip)', () => {
  let appdata: string;
  let actions: ActionManager;

  beforeAll(async () => {
    appdata = mkdtempSync(join(tmpdir(), 'solx-actions-test-'));
    const config = ConfigService.openIn(appdata);
    const types = await TypeManager.open(config.typesDbPath());
    const files = await FileStore.open(config.filesDir());
    const docs = await DocManager.open(
      config.docsDbPath(),
      `${config.searchIndexDir()}/docs`,
      types,
    );
    actions = await ActionManager.open(config.actionsDbPath(), config, types, docs, files);
  });

  afterAll(() => {
    if (appdata) rmSync(appdata, { recursive: true, force: true });
  });

  test('exec dispatches a seeded built-in action', async () => {
    const result = await actions.exec('/builtin', 'now', {});
    expect(result.success).toBe(true);
    expect(result.action).toBe('/builtin/now');
    const payload = result.result as { now: string };
    expect(payload.now).toMatch(/^\d{4}-\d{2}-\d{2}T/);
  }, 30_000);

  test('exec on an unknown action rejects with SolxError', async () => {
    await expect(actions.exec('/does/not', 'exist', {})).rejects.toThrow(SolxError);
  });

  test('save → get → list → delete a custom action', async () => {
    const saved = await actions.save('/tools', 'greet', {
      caption: 'Greet',
      description: 'A no-op command action, never executed in this test.',
      actionType: 'command',
      fnName: 'echo hello',
    });
    expect(saved.path).toBe('/tools');
    expect(saved.name).toBe('greet');
    expect(saved.actionType).toBe('command');
    expect(saved.trusted).toBe(false);
    expect(saved.id).toMatch(/^[0-9a-f-]{36}$/);

    const got = await actions.get('/tools', 'greet');
    expect(got.id).toBe(saved.id);
    expect(got.fnName).toBe('echo hello');

    const page = await actions.list({ pathPrefix: '/tools' });
    expect(page.items).toHaveLength(1);
    expect(page.items[0]?.id).toBe(saved.id);

    await actions.delete('/tools', 'greet');
    await expect(actions.get('/tools', 'greet')).rejects.toThrow(SolxError);
  }, 30_000);
});
