/**
 * Round-trip test for `createSolx` in **client (remote) mode**.
 *
 * Spins up a minimal in-process HTTP server that speaks the same wire
 * contract as `solx-server` (bearer-token auth; REST routes keyed on
 * method + `/<resource>/<ref>`, with raw bytes for file content) and points
 * `createSolx({ serverUrl, serverToken })` at it. This exercises the
 * Rust `solx_client::Remote*` path end-to-end through Neon — construct
 * via `.connect()`, dispatch a real HTTP round trip — without needing
 * to build/run the actual `solx-server` binary.
 *
 * Only the routes this test's assertions touch are implemented (one
 * round trip per manager); it is not a full mock of `solx-server`.
 *
 * Skips if the native binary is not built. Run
 * `cargo build -p solx-bindings` first, set `SOLX_NATIVE_BIN` to the
 * resulting `solx.node`, then run `bun run test`.
 */

import { afterAll, beforeAll, describe, expect, test } from 'vitest';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { randomUUID } from 'node:crypto';
import { createSolx } from '../src/index.js';
import type { Solx } from '../src/solx.js';

let nativePath: string | undefined;
const nativeEnv = process.env['SOLX_NATIVE_BIN'];
if (nativeEnv && existsSync(nativeEnv)) {
  nativePath = nativeEnv;
  process.env['SOLX_NODE_PATH'] = nativePath;
}

const describeIfNative = nativePath ? describe : describe.skip;

const TOKEN = 'test-bearer-token';
const NOW_ISO = '2026-01-01T00:00:00Z';

interface TypeEntitySnake {
  id: string;
  path: string;
  name: string;
  description?: string;
  schema: unknown;
  groups: string[];
  created_at: string;
  updated_at: string;
}

interface DocumentSnake {
  id: string;
  path: string;
  name: string;
  title?: string;
  summary?: string;
  type_ref: string;
  contents: unknown;
  author?: string;
  pub_date?: string;
  confidence?: number;
  links: unknown[];
  files: unknown[];
  created_at: string;
  updated_at: string;
}

function readBody(req: Parameters<Parameters<typeof createServer>[0]>[0]): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    req.on('data', (c: Buffer) => chunks.push(c));
    req.on('end', () => resolve(Buffer.concat(chunks)));
    req.on('error', reject);
  });
}

/**
 * Split a request path into its resource and the `(path, name)` the entity
 * routes carry in the URL — the client side of `solx_surface::path::split_ref`.
 * `/docs/notes/remote-note` -> `('docs', '/notes', 'remote-note')`;
 * `/docs/note` -> `('docs', '/', 'note')`.
 */
function splitUrl(url: string): { resource: string; path: string; name: string; rest: string } {
  const segments = url.split('?')[0]!.split('/').filter((s) => s.length > 0).map(decodeURIComponent);
  const resource = segments.shift() ?? '';
  const name = segments.pop() ?? '';
  const path = segments.length > 0 ? `/${segments.join('/')}` : '/';
  return { resource, path, name, rest: [...segments, name].join('/') };
}

/** Minimal stand-in for solx-server: bearer auth + a handful of REST routes. */
function startMockServer(): Promise<{ server: Server; url: string }> {
  const types = new Map<string, TypeEntitySnake>();
  const docs = new Map<string, DocumentSnake>();
  const files = new Map<string, Buffer>();

  return new Promise((resolve) => {
    const server = createServer((req, res) => {
      void (async () => {
        const auth = req.headers['authorization'];
        if (auth !== `Bearer ${TOKEN}`) {
          res.writeHead(401, { 'content-type': 'application/json' });
          res.end(JSON.stringify({ kind: 'other', message: 'unauthorized' }));
          return;
        }

        const raw = await readBody(req);
        const { resource, path, name, rest } = splitUrl(req.url ?? '');
        const method = req.method ?? 'GET';
        const json = () => JSON.parse(raw.toString('utf8') || '{}');
        const notFound = (what: string) => {
          res.writeHead(404, { 'content-type': 'application/json' });
          res.end(JSON.stringify({ kind: 'not_found', message: `${what} not found` }));
        };
        res.setHeader('content-type', 'application/json');

        // Files carry raw bytes rather than JSON, so they come first.
        if (resource === 'files') {
          if (method === 'PUT') {
            files.set(rest, raw);
            res.end(JSON.stringify({ rel_path: rest }));
            return;
          }
          if (method === 'GET') {
            const bytes = files.get(rest);
            if (bytes === undefined) return notFound('file');
            res.setHeader('content-type', 'application/octet-stream');
            res.end(bytes);
            return;
          }
        }

        const key = `${path === '/' ? '' : path}/${name}`;

        if (resource === 'types' && method === 'PUT') {
          const input = json();
          const entity: TypeEntitySnake = {
            id: types.get(key)?.id ?? randomUUID(),
            path,
            name,
            description: input.description,
            schema: input.schema,
            groups: input.groups ?? [],
            created_at: NOW_ISO,
            updated_at: NOW_ISO,
          };
          types.set(key, entity);
          res.end(JSON.stringify(entity));
          return;
        }
        if (resource === 'types' && method === 'GET') {
          const entity = types.get(key);
          if (!entity) return notFound('type');
          res.end(JSON.stringify(entity));
          return;
        }
        if (resource === 'docs' && method === 'PUT') {
          const input = json();
          const doc: DocumentSnake = {
            id: docs.get(key)?.id ?? randomUUID(),
            path,
            name,
            title: input.title,
            summary: input.summary,
            type_ref: input.type_ref,
            contents: input.contents,
            author: input.author,
            pub_date: input.pub_date,
            confidence: input.confidence,
            links: input.links ?? [],
            files: input.files ?? [],
            created_at: NOW_ISO,
            updated_at: NOW_ISO,
          };
          docs.set(key, doc);
          res.end(JSON.stringify(doc));
          return;
        }
        if (resource === 'docs' && method === 'GET') {
          const doc = docs.get(key);
          if (!doc) return notFound('doc');
          res.end(JSON.stringify(doc));
          return;
        }
        // POST on an action's own URL executes it, with the params as body.
        if (resource === 'actions' && method === 'POST') {
          res.end(
            JSON.stringify({
              action: key,
              result: { echoedParams: json() },
              success: true,
              message: null,
            }),
          );
          return;
        }

        res.writeHead(404);
        res.end(
          JSON.stringify({ kind: 'not_found', message: `no mock route for ${method} ${req.url}` }),
        );
      })();
    });
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address() as AddressInfo;
      resolve({ server, url: `http://127.0.0.1:${port}` });
    });
  });
}

describeIfNative('createSolx (client/remote mode round-trip)', () => {
  let appdata: string;
  let server: Server;
  let serverUrl: string;
  let solx: Solx;

  beforeAll(async () => {
    appdata = mkdtempSync(join(tmpdir(), 'solx-remote-test-'));
    const started = await startMockServer();
    server = started.server;
    serverUrl = started.url;
    // Explicit override takes precedence over env/config — forces
    // client mode without touching SOLX_SERVER_URL or the config file.
    solx = await createSolx({ appdata, serverUrl, serverToken: TOKEN });
  });

  afterAll(async () => {
    if (appdata) rmSync(appdata, { recursive: true, force: true });
    await new Promise<void>((resolve) => server.close(() => resolve()));
  });

  test('every manager was wired via .connect(), not .open()', () => {
    // A local open() would have created db/ and files/ under appdata
    // (LocalTypeManager/LocalDocManager/LocalActionManager each touch
    // their db file on open, LocalFileStore creates its root lazily
    // but earlier local-mode tests confirm the db/ dir appears
    // immediately); in remote mode nothing local should be touched
    // beyond the appdata directory itself (created eagerly by
    // ConfigService.openIn, which is always local).
    expect(existsSync(join(appdata, 'db'))).toBe(false);
    expect(existsSync(join(appdata, 'files'))).toBe(false);
    expect(existsSync(appdata)).toBe(true);
  });

  test('types.save → types.get round-trips through the mock server', async () => {
    const saved = await solx.types.save('/types/custom', 'Person', {
      schema: { type: 'object' },
      groups: ['g1'],
    });
    expect(saved.path).toBe('/types/custom');
    expect(saved.groups).toEqual(['g1']);

    const got = await solx.types.get('/types/custom', 'Person');
    expect(got.id).toBe(saved.id);
  });

  test('files.put → files.get round-trips through the mock server', async () => {
    const payload = new TextEncoder().encode('hello remote files');
    const putPath = await solx.files.put('files/docs/x/hello.txt', payload);
    expect(putPath).toBe('files/docs/x/hello.txt');

    const got = await solx.files.get('files/docs/x/hello.txt');
    expect(new TextDecoder().decode(got)).toBe('hello remote files');
  });

  test('docs.save → docs.get round-trips through the mock server', async () => {
    const saved = await solx.docs.save('/notes', 'remote-note', {
      typeRef: '/types/custom/Note',
      contents: { body: 'hi' },
    });
    expect(saved.typeRef).toBe('/types/custom/Note');

    const got = await solx.docs.get('/notes', 'remote-note');
    expect(got.id).toBe(saved.id);
  });

  test('actions.exec round-trips through the mock server', async () => {
    const result = await solx.actions.exec('/tools', 'echo', { hello: 'world' });
    expect(result.success).toBe(true);
    expect(result.action).toBe('/tools/echo');
    expect(result.result).toEqual({ echoedParams: { hello: 'world' } });
  });
});
