/**
 * Mirror of `solx_surface::entities` (see solx-core/solx-surface/src/entities.rs).
 *
 * These are plain DTOs. The Rust `#[derive(Serialize, Deserialize)]`
 * enums round-trip to the TS shapes below:
 *   - `LinkKind::DocRef | Url`          → `'doc_ref' | 'url'`
 *   - `ActionType::Wasm | Webhook | Command | Internal | Script`
 *                                      → `'wasm' | 'webhook' | 'command' | 'internal' | 'script'`
 *   - `SortOrder::Asc | Desc`          → `'asc' | 'desc'`
 *
 * UUIDs are hyphenated lowercase strings; timestamps are RFC 3339 strings.
 */

import type { JsonObject, JsonValue } from './json.js';

// ---------- LinkKind ----------

export type LinkKind = 'doc_ref' | 'url';
export const LinkKind = {
  DocRef: 'doc_ref',
  Url: 'url',
} as const;

// ---------- FileRef ----------

export interface FileRef {
  name: string;
  relPath: string;
  contentType?: string;
}

// ---------- DocLink ----------

export interface DocLink {
  kind: LinkKind;
  target: string;
  field?: string;
  title?: string;
  description?: string;
}

// ---------- TypeEntity + TypeInput ----------

export interface TypeEntity {
  id: string;
  path: string;
  name: string;
  description?: string;
  schema: JsonObject;
  groups: string[];
  createdAt: string;
  updatedAt: string;
}

export interface TypeInput {
  description?: string;
  schema: JsonObject;
  groups?: string[];
}

// ---------- Document + DocumentInput ----------

export interface Document {
  id: string;
  path: string;
  name: string;
  title?: string;
  summary?: string;
  typeRef: string;
  contents: JsonValue;
  author?: string;
  pubDate?: string;
  confidence?: number;
  links: DocLink[];
  files: FileRef[];
  createdAt: string;
  updatedAt: string;
}

export interface DocumentInput {
  title?: string;
  summary?: string;
  typeRef: string;
  contents: JsonValue;
  author?: string;
  pubDate?: string;
  confidence?: number;
  links?: DocLink[];
  files?: FileRef[];
}

// ---------- ActionType + Action + ActionInput ----------

export type ActionType = 'wasm' | 'webhook' | 'command' | 'internal' | 'script';
export const ActionType = {
  Wasm: 'wasm',
  Webhook: 'webhook',
  Command: 'command',
  Internal: 'internal',
  Script: 'script',
} as const;

export interface Action {
  id: string;
  path: string;
  name: string;
  caption?: string;
  description?: string;
  capabilities: string[];
  phrases: string[];
  category?: string;
  paramTypeRef?: string;
  resultTypeRef?: string;
  actionType?: ActionType;
  fnName?: string;
  binName?: string;
  actionConfig?: JsonObject;
  files: FileRef[];
  /** Historical trust flag — no longer affects WASM dispatch (every built-in moved to `Internal`), metadata only. */
  trusted: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ActionInput {
  caption?: string;
  description?: string;
  capabilities?: string[];
  phrases?: string[];
  category?: string;
  paramTypeRef?: string;
  resultTypeRef?: string;
  actionType?: ActionType;
  fnName?: string;
  binName?: string;
  actionConfig?: JsonObject;
  files?: FileRef[];
  trusted?: boolean;
}

// ---------- ActionExecResult ----------

export interface ActionExecResult {
  action: string;
  result: JsonValue;
  success: boolean;
  message?: string;
}

// ---------- WidgetDescriptor ----------

/** Returned by `widget_open` / `loopback::widget::open` (solx-actions). */
export interface WidgetDescriptor {
  widgetId: string;
  tagName: string;
  entryUrl: string;
  wsUrl: string;
  token: string;
  fields?: JsonValue;
}
