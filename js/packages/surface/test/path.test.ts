/**
 * Round-trip tests for the path helpers. These do not touch native code —
 * they verify the TS implementation matches the Rust semantics in
 * solx-core/solx-surface/src/path.rs.
 */

import { describe, expect, test } from 'vitest';
import { Name, Path, Ref, SolxError } from '../src/index.js';

describe('Path', () => {
  test('normalizes the empty / root', () => {
    expect(Path.normalize('')).toBe('/');
    expect(Path.normalize('/')).toBe('/');
  });

  test('adds a leading slash if missing', () => {
    expect(Path.normalize('foo/bar')).toBe('/foo/bar');
  });

  test('strips trailing slashes', () => {
    expect(Path.normalize('/foo/bar/')).toBe('/foo/bar');
  });

  test('rejects empty segments', () => {
    expect(() => Path.normalize('/foo//bar')).toThrow(SolxError);
  });

  test('rejects . and .. segments', () => {
    expect(() => Path.normalize('/foo/../bar')).toThrow(SolxError);
    expect(() => Path.normalize('/foo/./bar')).toThrow(SolxError);
  });

  test('isValid reflects normalize', () => {
    expect(Path.isValid('/foo/bar')).toBe(true);
    expect(Path.isValid('/foo//bar')).toBe(false);
  });
});

describe('Name', () => {
  test('rejects empty', () => {
    expect(() => Name.validate('')).toThrow(SolxError);
  });

  test('rejects slash', () => {
    expect(() => Name.validate('foo/bar')).toThrow(SolxError);
  });

  test('rejects whitespace and other chars', () => {
    expect(() => Name.validate('foo bar')).toThrow(SolxError);
    expect(() => Name.validate('foo!')).toThrow(SolxError);
  });

  test('accepts [A-Za-z0-9._-]+', () => {
    expect(Name.validate('hello')).toBe('hello');
    expect(Name.validate('Person.v2-x')).toBe('Person.v2-x');
  });
});

describe('Ref', () => {
  test('combines path + name', () => {
    expect(Ref.full('/research', 'note')).toBe('/research/note');
    expect(Ref.full('/', 'note')).toBe('/note');
  });

  test('splits a full ref', () => {
    expect(Ref.split('/research/note')).toEqual({
      path: '/research',
      name: 'note',
    });
    expect(Ref.split('/note')).toEqual({ path: '/', name: 'note' });
  });

  test('round-trips', () => {
    for (const ref of ['/a', '/a/b', '/a/b/c']) {
      const { path, name } = Ref.split(ref);
      expect(Ref.full(path, name)).toBe(ref);
    }
  });
});
