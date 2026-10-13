/**
 * Round-trip test for `@solx/scripts` end-to-end (local mode).
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
// `config`/`files`/`docs` are only needed here to build the
// `ActionManager` fixture `ScriptRunner` runs against — they aren't
// dependencies of `@solx/scripts` itself, so (matching the convention
// in packages/files/test/files.test.ts for cross-package test glue)
// they're imported by relative path rather than added to package.json.
import { ConfigService } from '../../config/src/index.js';
import { TypeManager } from '../../types/src/index.js';
import { FileStore } from '../../files/src/index.js';
import { DocManager } from '../../docs/src/index.js';
import { ActionManager, type ActionExecResult } from '@solx/actions';
import { ScriptRunner } from '../src/index.js';

let nativePath: string | undefined;
const nativeEnv = process.env['SOLX_NATIVE_BIN'];
if (nativeEnv && existsSync(nativeEnv)) {
  nativePath = nativeEnv;
  process.env['SOLX_NODE_PATH'] = nativePath;
}

const describeIfNative = nativePath ? describe : describe.skip;

describeIfNative('ScriptRunner (round-trip)', () => {
  let appdata: string;
  let scripts: ScriptRunner;

  beforeAll(async () => {
    appdata = mkdtempSync(join(tmpdir(), 'solx-scripts-test-'));
    const config = ConfigService.openIn(appdata);
    const types = await TypeManager.open(config.typesDbPath());
    const files = await FileStore.open(config.filesDir());
    const docs = await DocManager.open(
      config.docsDbPath(),
      `${config.searchIndexDir()}/docs`,
      types,
    );
    const actions = await ActionManager.open(config.actionsDbPath(), config, types, docs, files);
    scripts = ScriptRunner.for(actions);
  });

  afterAll(() => {
    if (appdata) rmSync(appdata, { recursive: true, force: true });
  });

  test('json stage returns the literal value', async () => {
    // A JSON *string* literal needs an outer quote layer for the
    // quote-aware tokenizer plus the JSON string's own inner quotes
    // (see solx-scripts' doc comment on `json '"..."'`); a bare
    // number needs neither since it has no embedded whitespace.
    expect(await scripts.run('json \'"hello"\'')).toBe('hello');
    expect(await scripts.run('json 42')).toBe(42);
  });

  test('exec stage dispatches to the action manager', async () => {
    const result = (await scripts.run('exec /builtin/now')) as unknown as ActionExecResult;
    expect(result.success).toBe(true);
    expect(result.action).toBe('/builtin/now');
    expect((result.result as { now: string }).now).toMatch(/^\d{4}-\d{2}-\d{2}T/);
  });

  test('a script returns its last statement\'s value', async () => {
    // The JSON object literal is single-quoted so the tokenizer
    // treats the internal whitespace as part of one token.
    const result = (await scripts.run(
      'json \'{"ignored": true}\' ; exec /builtin/now',
    )) as unknown as ActionExecResult;
    expect(result.action).toBe('/builtin/now');
  });

  test('$params is seeded and reachable via --json passthrough', async () => {
    const result = (await scripts.run(
      'exec /builtin/now --json $params',
      { note: 'unused by /builtin/now, just verifying substitution does not error' },
    )) as unknown as ActionExecResult;
    expect(result.success).toBe(true);
  });

  test('unsupported stage verbs reject with SolxError', async () => {
    await expect(scripts.run('get doc /a/b')).rejects.toThrow(SolxError);
  });
});
