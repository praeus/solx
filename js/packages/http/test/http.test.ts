/**
 * Exercises the REST surface `@solx/http` speaks, against an in-process
 * stand-in for `solx-server`. The assertions are about the *wire*: which
 * method and URL each manager call produces, what rides in the query string
 * versus the body, and how `204` and raw file bytes are handled. Those are
 * the parts a hand-written server (or a browser proxy) has to match, and the
 * parts that would silently drift from `solx-client`'s Rust equivalent.
 */
import { afterAll, beforeAll, describe, expect, test } from 'vitest';
import { createServer, type IncomingMessage, type ServerResponse, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { SolxError } from '@solx/surface';
import { connectHttp, type HttpSolx } from '../src/index.js';

const TOKEN = 'test-token';
const NOW = '2026-01-01T00:00:00Z';

interface Received {
  method: string;
  url: string;
  contentType?: string;
  body: Buffer;
}

let server: Server;
let solx: HttpSolx;
/** Every request the server saw, in order. */
const seen: Received[] = [];
/** Set by a test to override the next response. */
let nextResponse: ((res: ServerResponse) => void) | undefined;

function readBody(req: IncomingMessage): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    req.on('data', (c: Buffer) => chunks.push(c));
    req.on('end', () => resolve(Buffer.concat(chunks)));
    req.on('error', reject);
  });
}

beforeAll(async () => {
  server = createServer((req, res) => {
    void (async () => {
      const body = await readBody(req);
      seen.push({
        method: req.method ?? '',
        url: req.url ?? '',
        contentType: req.headers['content-type'],
        body,
      });

      if (req.headers['authorization'] !== `Bearer ${TOKEN}`) {
        res.writeHead(401, { 'content-type': 'application/json' });
        res.end(JSON.stringify({ kind: 'other', message: 'unauthorized' }));
        return;
      }

      if (nextResponse) {
        const respond = nextResponse;
        nextResponse = undefined;
        respond(res);
        return;
      }

      // Default: echo back a plausible entity so the converters run.
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(
        JSON.stringify({
          id: 'id-1',
          path: '/p',
          name: 'n',
          type_ref: '/types/docs/Document',
          contents: {},
          schema: {},
          groups: [],
          capabilities: [],
          phrases: [],
          links: [],
          files: [],
          created_at: NOW,
          updated_at: NOW,
        }),
      );
    })();
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address() as AddressInfo;
  solx = connectHttp(`http://127.0.0.1:${port}`, TOKEN);
});

afterAll(async () => {
  await new Promise<void>((resolve) => server.close(() => resolve()));
});

/** The most recent request the server received. */
function last(): Received {
  const r = seen[seen.length - 1];
  if (!r) throw new Error('no request recorded');
  return r;
}

describe('entity references in the URL', () => {
  test('a nested path becomes URL segments, with the input as the body', async () => {
    await solx.docs.save('/research/ai', 'note', {
      typeRef: '/types/docs/Document',
      contents: { k: 'v' },
    });
    expect(last().method).toBe('PUT');
    expect(last().url).toBe('/docs/research/ai/note');
    expect(JSON.parse(last().body.toString())).toEqual({
      type_ref: '/types/docs/Document',
      contents: { k: 'v' },
    });
  });

  test('the root path contributes no segments', async () => {
    await solx.docs.get('/', 'note');
    expect(last().method).toBe('GET');
    expect(last().url).toBe('/docs/note');
    expect(last().body).toHaveLength(0);
  });

  test('awkward characters in a name are percent-encoded', async () => {
    await solx.docs.get('/a b', '100% #1?');
    expect(last().url).toBe('/docs/a%20b/100%25%20%231%3F');
  });

  test('an empty or slash-bearing name is rejected before it is sent', async () => {
    // An empty name would collapse to a trailing slash, which the server
    // normalizes into a different, valid entity.
    await expect(solx.docs.get('/research/ai', '')).rejects.toThrow(SolxError);
    await expect(solx.docs.get('/research', 'a/b')).rejects.toThrow(SolxError);
  });

  test('each resource uses its own collection prefix', async () => {
    await solx.types.get('/types/core', 'String');
    expect(last().url).toBe('/types/types/core/String');
    await solx.actions.get('/tools', 'echo');
    expect(last().url).toBe('/actions/tools/echo');
  });
});

describe('collection routes', () => {
  test('list options ride in the query string, snake_cased', async () => {
    nextResponse = (res) => {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ items: [], total: 0, limit: 10, offset: 0 }));
    };
    await solx.docs.list({ pathPrefix: '/research', limit: 10, sortBy: 'created_at' });
    expect(last().method).toBe('GET');
    const url = new URL(`http://x${last().url}`);
    expect(url.pathname).toBe('/docs');
    expect(url.searchParams.get('path_prefix')).toBe('/research');
    expect(url.searchParams.get('limit')).toBe('10');
    expect(url.searchParams.get('sort_by')).toBe('created_at');
    expect(url.searchParams.has('offset')).toBe(false);
  });

  test('search is top-level, not a sibling of the docs catch-all', async () => {
    nextResponse = (res) => {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ hits: [], total: 0, limit: 10, offset: 0 }));
    };
    await solx.docs.search({ q: 'ada', typeRef: '/types/docs/Document' });
    const url = new URL(`http://x${last().url}`);
    expect(url.pathname).toBe('/search');
    expect(url.searchParams.get('q')).toBe('ada');
    expect(url.searchParams.get('type_ref')).toBe('/types/docs/Document');
  });

  test('validate is top-level, and a 204 resolves rather than throwing', async () => {
    nextResponse = (res) => res.writeHead(204).end();
    await expect(
      solx.types.validate({ name: 'Ada' }, '/types/custom/Person'),
    ).resolves.toBeUndefined();
    expect(last().method).toBe('POST');
    expect(last().url).toBe('/validate');
    expect(JSON.parse(last().body.toString())).toEqual({
      value: { name: 'Ada' },
      type_ref: '/types/custom/Person',
    });
  });

  test('resolve is a plain GET on the type, with no route of its own', async () => {
    await solx.types.resolve('/types/custom/Person');
    expect(last().method).toBe('GET');
    expect(last().url).toBe('/types/types/custom/Person');
  });
});

describe('actions', () => {
  test('exec POSTs the params to the action own URL', async () => {
    nextResponse = (res) => {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ action: '/tools/echo', result: 42, success: true }));
    };
    const result = await solx.actions.exec('/tools', 'echo', { hello: 'world' });
    expect(last().method).toBe('POST');
    expect(last().url).toBe('/actions/tools/echo');
    expect(JSON.parse(last().body.toString())).toEqual({ hello: 'world' });
    expect(result.success).toBe(true);
  });
});

describe('files', () => {
  test('put sends raw bytes, not base64 JSON', async () => {
    nextResponse = (res) => {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ rel_path: 'media/pic.png' }));
    };
    const bytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x00, 0xff]);
    const stored = await solx.files.put('media/pic.png', bytes);
    expect(stored).toBe('media/pic.png');
    expect(last().method).toBe('PUT');
    expect(last().url).toBe('/files/media/pic.png');
    expect(new Uint8Array(last().body)).toEqual(bytes);
    expect(last().contentType).not.toBe('application/json');
  });

  test('get reads raw bytes back, including non-UTF8 ones', async () => {
    const payload = Buffer.from([0x00, 0xff, 0xfe, 0x41]);
    nextResponse = (res) => {
      res.writeHead(200, { 'content-type': 'image/png' });
      res.end(payload);
    };
    const got = await solx.files.get('media/pic.png');
    expect(new Uint8Array(got)).toEqual(new Uint8Array(payload));
  });

  test('a byte view over a larger buffer is not over-sent', async () => {
    nextResponse = (res) => {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ rel_path: 'a.bin' }));
    };
    const backing = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);
    const view = backing.subarray(2, 5);
    await solx.files.put('a.bin', view);
    expect(new Uint8Array(last().body)).toEqual(new Uint8Array([3, 4, 5]));
  });

  test('list sends the prefix as a query parameter', async () => {
    nextResponse = (res) => {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ paths: ['media/pic.png'] }));
    };
    const paths = await solx.files.list('media');
    expect(paths).toEqual(['media/pic.png']);
    expect(last().url).toBe('/files?prefix=media');
  });
});

describe('errors', () => {
  test('a JSON error body becomes the matching SolxError variant', async () => {
    nextResponse = (res) => {
      res.writeHead(404, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ kind: 'not_found', message: 'no such doc' }));
    };
    await expect(solx.docs.get('/nope', 'nope')).rejects.toBeInstanceOf(SolxError);
  });

  test('an error on a raw-bytes route is still decoded as JSON', async () => {
    nextResponse = (res) => {
      res.writeHead(404, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ kind: 'not_found', message: 'no such file' }));
    };
    await expect(solx.files.get('gone.bin')).rejects.toBeInstanceOf(SolxError);
  });

  test('delete accepts an empty 204 body', async () => {
    nextResponse = (res) => res.writeHead(204).end();
    await expect(solx.docs.delete('/research/ai', 'note')).resolves.toBeUndefined();
    expect(last().method).toBe('DELETE');
    expect(last().url).toBe('/docs/research/ai/note');
  });
});
