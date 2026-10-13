/**
 * model.ts — the config editor's plumbing, independent of any one section.
 *
 * Each section edits its own draft shape (rows for a table, strings for
 * number inputs, ...), converts it back to `solx-config.json` keys, and the
 * editor sends only the keys whose value actually differs from the file —
 * `null` for a key that should go away. The host validates the whole result
 * before writing (`ConfigService::update`), so this side only has to get
 * the shape right.
 */
import type { ReactNode } from "react";
import type { ConfigChanges, ConfigView, SolxConfigFile } from "@solx/tauri";

export interface SectionEditorProps<D> {
  draft: D;
  setDraft: (next: D) => void;
  view: ConfigView;
}

export interface SectionDef<D> {
  id: string;
  title: string;
  description: ReactNode;
  /** Build the draft from the file as stored. */
  fromConfig: (raw: SolxConfigFile) => D;
  /**
   * The keys this section owns, as they should now be (`undefined` =
   * remove). May throw for a draft that can't be expressed (e.g. bad JSON).
   */
  toConfig: (draft: D, raw: SolxConfigFile) => Partial<Record<string, unknown>>;
  Editor: (props: SectionEditorProps<D>) => ReactNode;
  /** Read-only sections have no Save/Revert. */
  readOnly?: boolean;
}

/** JSON with object keys sorted, so key order never reads as a change. */
export function stableStringify(value: unknown): string {
  return JSON.stringify(value ?? null, (_key, v: unknown) => {
    if (v && typeof v === "object" && !Array.isArray(v)) {
      return Object.fromEntries(
        Object.entries(v as Record<string, unknown>).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
      );
    }
    return v;
  });
}

/** The subset of `proposed` that differs from `raw`, with removals as `null`. */
export function diffChanges(
  raw: SolxConfigFile,
  proposed: Partial<Record<string, unknown>>,
): ConfigChanges {
  const changes: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(proposed)) {
    const next = value === undefined ? null : value;
    if (stableStringify(next) !== stableStringify(raw[key] ?? null)) {
      changes[key] = next;
    }
  }
  return changes as ConfigChanges;
}

/** `undefined` for an empty list/map, so an emptied setting is removed rather than stored empty. */
export function nonEmpty<T extends unknown[] | Record<string, unknown>>(value: T): T | undefined {
  return (Array.isArray(value) ? value.length : Object.keys(value).length) ? value : undefined;
}

/** Number input text → value: blank means "unset" (use the default). */
export function parseNumberInput(text: string): number | undefined | string {
  const t = text.trim();
  if (!t) return undefined;
  const n = Number(t);
  // Left as text on purpose: the host rejects it with a clear type error.
  return Number.isInteger(n) ? n : t;
}

/**
 * The same glob the host uses for tool rules (`solx-config` `glob_matches`):
 * `*` matches anything including `/`, `?` exactly one character.
 */
export function globMatches(pattern: string, text: string): boolean {
  const escaped = pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".");
  return new RegExp(`^${escaped}$`, "s").test(text);
}

/** True when an address is loopback (or unset, which means 127.0.0.1). */
export function isLoopbackHost(host: string | undefined): boolean {
  const h = (host ?? "").trim();
  return h === "" || h === "localhost" || h === "::1" || h.startsWith("127.");
}
