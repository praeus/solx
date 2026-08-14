/**
 * Round-trip test for `@solx/docs` end-to-end (local mode).
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
import { TypeManager } from '@solx/types';
import { DocManager } from '../src/index.js';

let nativePath: string | undefined;
const nativeEnv = process.env['SOLX_NATIVE_BIN'];
if (nativeEnv && existsSync(nativeEnv)) {
  nativePath = nativeEnv;
  process.env['SOLX_NODE_PATH'] = nativePath;
}

const describeIfNative = nativePath ? describe : describe.skip;

describeIfNative('DocManager (round-trip)', () => {
  let appdata: string;
  let types: TypeManager;
  let docs: DocManager;

  beforeAll(async () => {
    appdata = mkdtempSync(join(tmpdir(), 'solx-docs-test-'));
    types = await TypeManager.open(join(appdata, 'types.db'));
    await types.save('/types/custom', 'Note', {
      schema: { type: 'object', properties: { body: { type: 'string' } } },
    });
    docs = await DocManager.open(
      join(appdata, 'docs.db'),
      join(appdata, 'search_index', 'docs'),
      types,
    );
  });

  afterAll(() => {
    if (appdata) rmSync(appdata, { recursive: true, force: true });
  });

  test('save → get → list → search → delete', async () => {
    const saved = await docs.save('/notes', 'first', {
      typeRef: '/types/custom/Note',
      title: 'First note',
      contents: { body: 'hello docs' },
    });
    expect(saved.path).toBe('/notes');
    expect(saved.name).toBe('first');
    expect(saved.typeRef).toBe('/types/custom/Note');
    expect(saved.title).toBe('First note');
    expect(saved.id).toMatch(/^[0-9a-f-]{36}$/);

    const got = await docs.get('/notes', 'first');
    expect(got.id).toBe(saved.id);
    expect(got.contents).toEqual({ body: 'hello docs' });

    const page = await docs.list({ pathPrefix: '/notes' });
    expect(page.items).toHaveLength(1);
    expect(page.items[0]?.id).toBe(saved.id);

    const results = await docs.search({ q: 'hello', pathPrefix: '/notes' });
    expect(results.hits.some((h) => h.id === saved.id)).toBe(true);

    await docs.delete('/notes', 'first');
    await expect(docs.get('/notes', 'first')).rejects.toThrow(SolxError);
  }, 30_000);

  test('save rejects a document that fails schema validation against a stricter type', async () => {
    await types.save('/types/custom', 'Strict', {
      schema: {
        type: 'object',
        required: ['body'],
        properties: { body: { type: 'string' } },
      },
    });
    await expect(
      docs.save('/notes', 'bad', {
        typeRef: '/types/custom/Strict',
        contents: {},
      }),
    ).rejects.toThrow(SolxError);
  });
});
