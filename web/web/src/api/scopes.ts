/**
 * scopes.ts — the access presets a new token can be given, and how a
 * token's scope strings are shown.
 *
 * The expansions mirror `Preset::scopes` in solx-surface's scope.rs, in the
 * canonical form the server returns (`read:document` is every document,
 * `exec:/*` every action). A preset is expanded when the token is issued,
 * so these are only for recognising one again in a list.
 */
import type { ScopePreset } from "@solx/tauri";

export interface PresetInfo {
  id: ScopePreset;
  label: string;
  hint: string;
  scopes: string[];
}

const READ_ALL = ["read:document", "read:type", "read:action", "read:file"];

export const PRESETS: readonly PresetInfo[] = [
  {
    id: "full",
    label: "Full access",
    hint: "Everything except what stays with this machine: managing devices, secrets and locked actions.",
    scopes: ["all"],
  },
  {
    id: "editor",
    label: "Editor",
    hint: "Read and write documents, types and files, and read actions. Can't run or change actions.",
    scopes: [...READ_ALL, "write:document", "write:type", "write:file"],
  },
  {
    id: "tools",
    label: "Tools",
    hint: "List and run actions, e.g. for an MCP client. Each action acts with its own scopes.",
    scopes: ["read:action", "exec:/*"],
  },
  {
    id: "read-only",
    label: "Read only",
    hint: "Read documents, types, actions and files. Can't change or run anything.",
    scopes: READ_ALL,
  },
];

/** The preset `scopes` is exactly, if any. */
export function presetOf(scopes: readonly string[]): PresetInfo | null {
  const have = new Set(scopes);
  return (
    PRESETS.find((p) => p.scopes.length === have.size && p.scopes.every((s) => have.has(s))) ?? null
  );
}

/**
 * One line for a token's access: a preset's name, or the scope strings.
 * `null` is the root token, which no scope limits.
 */
export function describeScopes(scopes: readonly string[] | null): string {
  if (scopes === null) return "Unrestricted (this machine's own token)";
  const preset = presetOf(scopes);
  if (preset) return preset.label;
  if (!scopes.length) return "Nothing";
  return scopes.join(", ");
}

/**
 * Suggestions for typing a scope. A resource scope takes a path after
 * another `:` (`read:document:/notes/*`), and `exec:` always takes one.
 */
export const SCOPE_SUGGESTIONS: readonly string[] = [
  "all",
  ...["document", "type", "action", "file"].flatMap((r) => [`read:${r}`, `write:${r}`]),
  "write:script",
  "write:wasm",
  "write:command",
  "write:webhook",
  "exec:/*",
  "exec:/builtin/*",
  "exec:/packages/*",
];
