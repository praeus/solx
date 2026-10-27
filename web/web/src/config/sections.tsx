/**
 * sections.tsx — one `SectionDef` per group of solx-config.json keys. Each
 * owns a draft shape convenient for its form and converts it back to file
 * keys; `ConfigApp` does the diffing, saving and restart prompting.
 *
 * Which keys need a restart comes from the host (`view.restartKeys`), not
 * from here — see `RESTART_KEYS` in solx-config.
 */
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type {
  ClientInfo,
  ConfigView,
  ConnectCode,
  ExclusionLists,
  PackageResult,
  ScopePreset,
  SolxConfigFile,
  TlsCert,
  TlsResult,
  TlsStatus,
  ToolRule,
} from "@solx/tauri";
import { ChipTagInput } from "../components/ChipTagInput";
import { JsonEditField } from "../components/JsonEditField";
import { getTauri, restartApp, revealCertsFolder } from "../api/connection";
import { useActionPaths, useDocumentPaths, useTypePaths } from "../api/hooks";
import { describeScopes, PRESETS, SCOPE_SUGGESTIONS } from "../api/scopes";
import {
  globMatches,
  isLoopbackHost,
  nonEmpty,
  parseNumberInput,
  type SectionDef,
  type SectionEditorProps,
} from "./model";

// ── shared bits ─────────────────────────────────────────────────────────

function RestartTag({ view, keys }: { view: ConfigView; keys: string[] }) {
  if (!keys.some((k) => view.restartKeys.includes(k))) return null;
  return (
    <span className="config-tag" title="Read when solx starts: takes effect after a restart">
      restart
    </span>
  );
}

function Field({
  label,
  hint,
  view,
  keys = [],
  children,
}: {
  label: string;
  hint?: ReactNode;
  view: ConfigView;
  keys?: string[];
  children: ReactNode;
}) {
  return (
    <label className="dialog__field">
      <span className="dialog__label">
        {label} <RestartTag view={view} keys={keys} />
      </span>
      {children}
      {hint && <span className="dialog__hint">{hint}</span>}
    </label>
  );
}

function TextInput(props: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  mono?: boolean;
  type?: string;
}) {
  return (
    <input
      className={`dialog__input${props.mono ? " config-mono" : ""}`}
      type={props.type ?? "text"}
      value={props.value}
      placeholder={props.placeholder}
      spellCheck={false}
      onChange={(e) => props.onChange(e.target.value)}
    />
  );
}

function RemoveButton({ onClick, label }: { onClick: () => void; label: string }) {
  return (
    <button type="button" className="config-icon-btn" onClick={onClick} aria-label={label} title={label}>
      ✕
    </button>
  );
}

function AddButton({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return (
    <button type="button" className="dialog__btn config-add-btn" onClick={onClick}>
      + {children}
    </button>
  );
}

function replaceAt<T>(list: T[], index: number, item: T): T[] {
  return list.map((x, i) => (i === index ? item : x));
}

function removeAt<T>(list: T[], index: number): T[] {
  return list.filter((_, i) => i !== index);
}

/** Who granted what, from `installed_packages`, for "granted by" labels. */
function grants(raw: SolxConfigFile, field: "granted_commands" | "granted_base_urls"): Map<string, string[]> {
  const out = new Map<string, string[]>();
  for (const pkg of raw.installed_packages ?? []) {
    for (const item of pkg[field] ?? []) out.set(item, [...(out.get(item) ?? []), pkg.name]);
  }
  return out;
}

// ── allowlists ──────────────────────────────────────────────────────────

interface CommandRow {
  key: string;
  command: string;
  description: string;
  cwd: string;
}

interface AllowlistDraft {
  urls: string[];
  commands: CommandRow[];
}

function AllowlistEditor({ draft, setDraft, view }: SectionEditorProps<AllowlistDraft>) {
  const urlGrants = useMemo(() => grants(view.raw, "granted_base_urls"), [view.raw]);
  const cmdGrants = useMemo(() => grants(view.raw, "granted_commands"), [view.raw]);
  const packagedUrls = draft.urls.filter((u) => urlGrants.has(u));

  return (
    <>
      <Field
        label="Allowed outbound URL prefixes"
        view={view}
        keys={["allowed_base_urls"]}
        hint={
          <>
            Webhook actions and the <code>/builtin/web/*</code> built-ins may only call URLs under one of these
            (same scheme, host and port; <code>http://host:</code> for any port). Empty means no outbound
            requests at all.
            {packagedUrls.length > 0 && (
              <> Granted by packages: {packagedUrls.map((u) => `${u} (${urlGrants.get(u)!.join(", ")})`).join("; ")}.</>
            )}
          </>
        }
      >
        <ChipTagInput
          chips={draft.urls}
          onChange={(urls) => setDraft({ ...draft, urls })}
          placeholder="https://api.example.com"
        />
      </Field>

      <div className="dialog__field">
        <span className="dialog__label">
          Command actions <RestartTag view={view} keys={["command_actions"]} />
        </span>
        <span className="dialog__hint">
          The only shell commands <code>Command</code> actions can run. An action names a key; the command
          string here is what actually runs, and this <code>cwd</code> overrides the action's own. Empty means
          no Command action can run.
        </span>
        <div className="config-rows">
          {draft.commands.map((row, i) => (
            <div className="config-card" key={i}>
              <div className="dialog__input-row">
                <TextInput
                  value={row.key}
                  placeholder="key (what actions reference)"
                  mono
                  onChange={(key) => setDraft({ ...draft, commands: replaceAt(draft.commands, i, { ...row, key }) })}
                />
                {cmdGrants.has(row.key) && (
                  <span className="config-tag config-tag--muted">{cmdGrants.get(row.key)!.join(", ")}</span>
                )}
                <RemoveButton
                  label="Remove command"
                  onClick={() => setDraft({ ...draft, commands: removeAt(draft.commands, i) })}
                />
              </div>
              <TextInput
                value={row.command}
                placeholder="command to run"
                mono
                onChange={(command) =>
                  setDraft({ ...draft, commands: replaceAt(draft.commands, i, { ...row, command }) })
                }
              />
              <div className="dialog__input-row">
                <TextInput
                  value={row.description}
                  placeholder="description (optional)"
                  onChange={(description) =>
                    setDraft({ ...draft, commands: replaceAt(draft.commands, i, { ...row, description }) })
                  }
                />
                <TextInput
                  value={row.cwd}
                  placeholder="working directory (optional)"
                  mono
                  onChange={(cwd) => setDraft({ ...draft, commands: replaceAt(draft.commands, i, { ...row, cwd }) })}
                />
              </div>
            </div>
          ))}
          <AddButton
            onClick={() =>
              setDraft({ ...draft, commands: [...draft.commands, { key: "", command: "", description: "", cwd: "" }] })
            }
          >
            Add command
          </AddButton>
        </div>
      </div>
    </>
  );
}

const allowlists: SectionDef<AllowlistDraft> = {
  id: "allowlists",
  title: "Allowlists",
  description:
    "What actions may reach outside solx. Both lists deny by default, and actions, MCP tools and scripts can never change them; only you can, here or in the file.",
  fromConfig: (raw) => ({
    urls: [...new Set([...(raw.allowed_base_urls ?? []), ...(raw.allowed_webhook_base_urls ?? [])])],
    commands: Object.entries(raw.command_actions ?? {}).map(([key, def]) => ({
      key,
      command: def.command,
      description: def.description ?? "",
      cwd: def.cwd ?? "",
    })),
  }),
  toConfig: (draft) => ({
    allowed_base_urls: nonEmpty(draft.urls.map((u) => u.trim()).filter(Boolean)),
    allowed_webhook_base_urls: undefined,
    command_actions: nonEmpty(
      Object.fromEntries(
        draft.commands.map((r) => [
          r.key.trim(),
          {
            command: r.command,
            ...(r.description.trim() ? { description: r.description.trim() } : {}),
            ...(r.cwd.trim() ? { cwd: r.cwd.trim() } : {}),
          },
        ]),
      ),
    ),
  }),
  Editor: AllowlistEditor,
};

// ── tool policy ─────────────────────────────────────────────────────────

interface RuleRow {
  path: string;
  actions: string[];
}

interface ToolDraft {
  exclude: RuleRow[];
  destructive: RuleRow[];
}

const toRows = (rules: ToolRule[] | undefined): RuleRow[] =>
  (rules ?? []).map((r) => ({ path: r.path, actions: r.actions ?? [] }));

const toRules = (rows: RuleRow[]): ToolRule[] | undefined =>
  nonEmpty(rows.map((r) => ({ path: r.path.trim(), ...(r.actions.length ? { actions: r.actions } : {}) })));

function RuleList({
  rows,
  onChange,
  paths,
  addLabel,
}: {
  rows: RuleRow[];
  onChange: (rows: RuleRow[]) => void;
  paths: string[];
  addLabel: string;
}) {
  return (
    <div className="config-rows">
      {rows.map((row, i) => {
        const matched = row.path.trim() ? paths.filter((p) => globMatches(row.path.trim(), p)).length : 0;
        return (
          <div className="config-card" key={i}>
            <div className="dialog__input-row">
              <input
                className="dialog__input config-mono"
                list="config-action-paths"
                value={row.path}
                placeholder="/packages/example/*"
                spellCheck={false}
                onChange={(e) => onChange(replaceAt(rows, i, { ...row, path: e.target.value }))}
              />
              <span className="config-tag config-tag--muted" title="Action paths this glob matches right now">
                {matched} {matched === 1 ? "path" : "paths"}
              </span>
              <RemoveButton label="Remove rule" onClick={() => onChange(removeAt(rows, i))} />
            </div>
            <ChipTagInput
              chips={row.actions}
              onChange={(actions) => onChange(replaceAt(rows, i, { ...row, actions }))}
              placeholder="only these action names (empty = every action under the path)"
            />
          </div>
        );
      })}
      <AddButton onClick={() => onChange([...rows, { path: "", actions: [] }])}>{addLabel}</AddButton>
    </div>
  );
}

function ToolEditor({ draft, setDraft, view }: SectionEditorProps<ToolDraft>) {
  // Unfiltered, so an action path that is already hidden can still be picked.
  const pathsQuery = useActionPaths(500, { excludeMode: "none" });
  const paths = useMemo(() => (pathsQuery.data?.items ?? []).map((f) => f.path), [pathsQuery.data]);
  return (
    <>
      <datalist id="config-action-paths">
        {paths.map((p) => (
          <option key={p} value={p} />
        ))}
      </datalist>
      <div className="dialog__field">
        <span className="dialog__label">
          Hidden from models <RestartTag view={view} keys={["tool_exclude"]} />
        </span>
        <span className="dialog__hint">
          Left out of the tool catalogue a model sees, and refused if called by name. Paths are globs:{" "}
          <code>*</code> spans <code>/</code>, <code>?</code> is one character. Actions can also hide themselves
          with the <code>solx:hidden</code> tag, but can't unhide what's listed here. These rules join the
          tooling action patterns under <em>Exclusions</em>.
        </span>
        <RuleList
          rows={draft.exclude}
          onChange={(exclude) => setDraft({ ...draft, exclude })}
          paths={paths}
          addLabel="Add hidden rule"
        />
      </div>
      <div className="dialog__field">
        <span className="dialog__label">
          Destructive <RestartTag view={view} keys={["tool_destructive"]} />
        </span>
        <span className="dialog__hint">
          Callable, but a client with an approval step asks first. Command and Webhook actions are always
          destructive.
        </span>
        <RuleList
          rows={draft.destructive}
          onChange={(destructive) => setDraft({ ...draft, destructive })}
          paths={paths}
          addLabel="Add destructive rule"
        />
      </div>
    </>
  );
}

const tools: SectionDef<ToolDraft> = {
  id: "tools",
  title: "Tool policy",
  description: "Which actions a model can see, and which need approval before it runs them.",
  fromConfig: (raw) => ({
    exclude: [...toRows(raw.tool_exclude), ...toRows(raw.mcp_exclude)],
    destructive: toRows(raw.tool_destructive),
  }),
  toConfig: (draft) => ({
    tool_exclude: toRules(draft.exclude),
    mcp_exclude: undefined,
    tool_destructive: toRules(draft.destructive),
  }),
  Editor: ToolEditor,
};

// ── exclusions ──────────────────────────────────────────────────────────

const SCOPES = [
  {
    key: "user",
    title: "Hidden from the web UI",
    hint: "Left out of every list, search and path picker here and over REST, and read as not found if opened directly. Models still see them — use this for agent skills and sessions.",
  },
  {
    key: "tooling",
    title: "Hidden from models",
    hint: "Left out of what MCP clients and the agent can list, search or read, and refused if addressed directly. The web UI still shows them.",
  },
] as const;

const KINDS = [
  { key: "documents", label: "Documents" },
  { key: "types", label: "Types" },
  { key: "actions", label: "Actions" },
] as const;

type ScopeKey = (typeof SCOPES)[number]["key"];
type KindKey = (typeof KINDS)[number]["key"];
type ExclusionDraft = Record<ScopeKey, Record<KindKey, string[]>>;

/**
 * Whether an exclusion pattern hides anything at `path`. The host also
 * matches a pattern against an entity's full reference, which a path facet
 * can't show, so this counts folders only - a hint, not the rule.
 */
const patternHitsPath = (pattern: string, path: string) =>
  globMatches(pattern.replace(/\/+$/, "") || "/", path);

function PatternList({
  patterns,
  onChange,
  paths,
  listId,
}: {
  patterns: string[];
  onChange: (next: string[]) => void;
  paths: string[];
  listId: string;
}) {
  return (
    <div className="config-rows">
      {patterns.map((pattern, i) => {
        const p = pattern.trim();
        const matched = p ? paths.filter((path) => patternHitsPath(p, path)).length : 0;
        return (
          <div className="dialog__input-row" key={i}>
            <input
              className="dialog__input config-mono"
              list={listId}
              value={pattern}
              placeholder="/agent/skills or /tmp*"
              spellCheck={false}
              onChange={(e) => onChange(replaceAt(patterns, i, e.target.value))}
            />
            <span className="config-tag config-tag--muted" title="Folders this pattern covers right now">
              {matched} {matched === 1 ? "folder" : "folders"}
            </span>
            <RemoveButton label="Remove pattern" onClick={() => onChange(removeAt(patterns, i))} />
          </div>
        );
      })}
      <AddButton onClick={() => onChange([...patterns, ""])}>Add pattern</AddButton>
    </div>
  );
}

function ExclusionsEditor({ draft, setDraft }: SectionEditorProps<ExclusionDraft>) {
  // Unfiltered, so a folder that is already excluded still shows up here.
  const docs = useDocumentPaths(500, { excludeMode: "none" });
  const types = useTypePaths(500, { excludeMode: "none" });
  const actions = useActionPaths(500, { excludeMode: "none" });
  const paths: Record<KindKey, string[]> = useMemo(
    () => ({
      documents: (docs.data?.items ?? []).map((f) => f.path),
      types: (types.data?.items ?? []).map((f) => f.path),
      actions: (actions.data?.items ?? []).map((f) => f.path),
    }),
    [docs.data, types.data, actions.data],
  );
  return (
    <>
      {KINDS.map((kind) => (
        <datalist id={`config-exclusion-${kind.key}`} key={kind.key}>
          {paths[kind.key].map((p) => (
            <option key={p} value={p} />
          ))}
        </datalist>
      ))}
      <span className="dialog__hint">
        Patterns are globs: <code>*</code> spans <code>/</code>, <code>?</code> is one character. A pattern
        matches a folder or a single entity: <code>/agent/skills</code> hides what's in that folder and the
        entity <code>skills</code> in <code>/agent</code>; <code>/agent/skills*</code> also hides everything
        beneath it.
      </span>
      {SCOPES.map((scope) => (
        <div className="dialog__field" key={scope.key}>
          <span className="dialog__label">{scope.title}</span>
          <span className="dialog__hint">{scope.hint}</span>
          {KINDS.map((kind) => (
            <div className="config-card" key={kind.key}>
              <span className="dialog__label">{kind.label}</span>
              <PatternList
                patterns={draft[scope.key][kind.key]}
                paths={paths[kind.key]}
                listId={`config-exclusion-${kind.key}`}
                onChange={(next) =>
                  setDraft({ ...draft, [scope.key]: { ...draft[scope.key], [kind.key]: next } })
                }
              />
            </div>
          ))}
        </div>
      ))}
    </>
  );
}

const toLists = (lists: ExclusionLists | undefined): Record<KindKey, string[]> => ({
  documents: [...(lists?.documents ?? [])],
  types: [...(lists?.types ?? [])],
  actions: [...(lists?.actions ?? [])],
});

/** Blank rows dropped, empty kinds and scopes omitted, so nothing empty is stored. */
const fromLists = (lists: Record<KindKey, string[]>): ExclusionLists | undefined => {
  const out: ExclusionLists = {};
  for (const { key } of KINDS) {
    const kept = nonEmpty(lists[key].map((p) => p.trim()).filter(Boolean));
    if (kept) out[key] = kept;
  }
  return Object.keys(out).length ? out : undefined;
};

const exclusions: SectionDef<ExclusionDraft> = {
  id: "exclusions",
  title: "Exclusions",
  description: "Documents, types and actions to hide from the web UI, from models, or from both.",
  fromConfig: (raw) => ({
    user: toLists(raw.exclusions?.user),
    tooling: toLists(raw.exclusions?.tooling),
  }),
  toConfig: (draft) => {
    const user = fromLists(draft.user);
    const tooling = fromLists(draft.tooling);
    return { exclusions: user || tooling ? { ...(user && { user }), ...(tooling && { tooling }) } : undefined };
  },
  Editor: ExclusionsEditor,
};

// ── runtime limits ──────────────────────────────────────────────────────

const LIMITS = [
  ["console_max_entries", "Console entries kept per action", "Oldest entries are dropped past this."],
  ["console_ttl_days", "Console retention (days)", "Consoles idle this long are deleted at startup."],
  ["background_timeout_secs", "Background action timeout (s)", "For action-start runs with no timeout of their own."],
  ["stop_grace_secs", "Stop grace period (s)", "How long action-stop waits before force-aborting."],
  ["invocation_ttl_days", "Finished invocation retention (days)", "Older finished runs are deleted at startup."],
  ["http_stream_idle_ttl_secs", "Idle HTTP stream timeout (s)", "A stream nobody polls for this long stops."],
  ["http_stream_max_buffer_bytes", "HTTP stream buffer (bytes)", "Oldest chunks are dropped past this."],
] as const;

type LimitKey = (typeof LIMITS)[number][0];
type LimitDraft = Record<LimitKey, string>;

function LimitEditor({ draft, setDraft, view }: SectionEditorProps<LimitDraft>) {
  return (
    <div className="config-grid">
      {LIMITS.map(([key, label, hint]) => (
        <Field key={key} label={label} hint={hint} view={view} keys={[key]}>
          <TextInput
            type="number"
            value={draft[key]}
            placeholder={`${view.effective[key]} (default)`}
            onChange={(v) => setDraft({ ...draft, [key]: v })}
          />
        </Field>
      ))}
    </div>
  );
}

const limits: SectionDef<LimitDraft> = {
  id: "limits",
  title: "Runtime limits",
  description: "Timeouts, retention and buffer sizes. Leave a field blank to use the default shown.",
  fromConfig: (raw) =>
    Object.fromEntries(LIMITS.map(([key]) => [key, raw[key] == null ? "" : String(raw[key])])) as LimitDraft,
  toConfig: (draft) => Object.fromEntries(LIMITS.map(([key]) => [key, parseNumberInput(draft[key])])),
  Editor: LimitEditor,
};

// ── environment ─────────────────────────────────────────────────────────

interface EnvDraft {
  mappings: Array<{ key: string; variable: string }>;
  vars: Array<{ namespace: string; key: string; value: string }>;
}

function EnvEditor({ draft, setDraft, view }: SectionEditorProps<EnvDraft>) {
  return (
    <>
      <div className="dialog__field">
        <span className="dialog__label">
          Environment mappings <RestartTag view={view} keys={["env_mappings"]} />
        </span>
        <span className="dialog__hint">
          The only system environment variables actions can read through <code>get-env</code>, each under the
          name an action asks for.
        </span>
        <div className="config-rows">
          {draft.mappings.map((row, i) => (
            <div className="dialog__input-row" key={i}>
              <TextInput
                value={row.key}
                placeholder="name actions use"
                mono
                onChange={(key) => setDraft({ ...draft, mappings: replaceAt(draft.mappings, i, { ...row, key }) })}
              />
              <span className="config-arrow">→</span>
              <TextInput
                value={row.variable}
                placeholder="SYSTEM_ENV_VAR"
                mono
                onChange={(variable) =>
                  setDraft({ ...draft, mappings: replaceAt(draft.mappings, i, { ...row, variable }) })
                }
              />
              <RemoveButton
                label="Remove mapping"
                onClick={() => setDraft({ ...draft, mappings: removeAt(draft.mappings, i) })}
              />
            </div>
          ))}
          <AddButton onClick={() => setDraft({ ...draft, mappings: [...draft.mappings, { key: "", variable: "" }] })}>
            Add mapping
          </AddButton>
        </div>
      </div>

      <div className="dialog__field">
        <span className="dialog__label">
          Persisted variables <RestartTag view={view} keys={["env_vars"]} />
        </span>
        <div className="config-warning">
          Stored in plain text in solx-config.json. Put anything sensitive in a secret (
          <code>set-secret</code>) instead, which is encrypted in the OS credential store.
        </div>
        <span className="dialog__hint">
          Written by <code>set-env</code> with <code>persist: true</code>.
        </span>
        <div className="config-rows">
          {draft.vars.map((row, i) => (
            <div className="dialog__input-row" key={i}>
              <TextInput
                value={row.namespace}
                placeholder="namespace"
                mono
                onChange={(namespace) => setDraft({ ...draft, vars: replaceAt(draft.vars, i, { ...row, namespace }) })}
              />
              <TextInput
                value={row.key}
                placeholder="key"
                mono
                onChange={(key) => setDraft({ ...draft, vars: replaceAt(draft.vars, i, { ...row, key }) })}
              />
              <TextInput
                value={row.value}
                placeholder="value"
                onChange={(value) => setDraft({ ...draft, vars: replaceAt(draft.vars, i, { ...row, value }) })}
              />
              <RemoveButton
                label="Remove variable"
                onClick={() => setDraft({ ...draft, vars: removeAt(draft.vars, i) })}
              />
            </div>
          ))}
          <AddButton
            onClick={() => setDraft({ ...draft, vars: [...draft.vars, { namespace: "", key: "", value: "" }] })}
          >
            Add variable
          </AddButton>
        </div>
      </div>
    </>
  );
}

const environment: SectionDef<EnvDraft> = {
  id: "environment",
  title: "Environment",
  description: "What actions can read from the environment store.",
  fromConfig: (raw) => ({
    mappings: Object.entries(raw.env_mappings ?? {}).map(([key, variable]) => ({ key, variable })),
    vars: Object.entries(raw.env_vars ?? {}).flatMap(([namespace, kv]) =>
      Object.entries(kv).map(([key, value]) => ({ namespace, key, value })),
    ),
  }),
  toConfig: (draft) => {
    const vars: Record<string, Record<string, string>> = {};
    for (const { namespace, key, value } of draft.vars) {
      if (!namespace.trim() || !key.trim()) throw new Error("Every persisted variable needs a namespace and a key.");
      (vars[namespace.trim()] ??= {})[key.trim()] = value;
    }
    const mappings: Record<string, string> = {};
    for (const { key, variable } of draft.mappings) {
      if (!key.trim() || !variable.trim()) throw new Error("Every environment mapping needs both names.");
      mappings[key.trim()] = variable.trim();
    }
    return { env_mappings: nonEmpty(mappings), env_vars: nonEmpty(vars) };
  },
  Editor: EnvEditor,
};

// ── HTTP server ─────────────────────────────────────────────────────────

/** `server_tls` as the form offers it; `custom` is a files or store value, kept as is. */
type TlsChoice = "default" | "auto" | "off" | "custom";

/** The desktop host turns TLS on for a network address on Windows when `server_tls` is unset. */
const IS_WINDOWS = typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);

interface ServerDraft {
  embedded: boolean;
  tls: TlsChoice;
  tlsCustom: SolxConfigFile["server_tls"] | null;
  host: string;
  port: string;
  origins: string[];
  publicUrl: string;
  tokenDays: string;
}

function ServerEditor({ draft, setDraft, view }: SectionEditorProps<ServerDraft>) {
  return (
    <>
      <label className="config-check">
        <input
          type="checkbox"
          checked={draft.embedded}
          onChange={(e) => setDraft({ ...draft, embedded: e.target.checked })}
        />
        <span>
          Serve solx over HTTP from this app <RestartTag view={view} keys={["server_embedded"]} />
        </span>
      </label>
      <span className="dialog__hint">
        For browsers, solx-cli, desktop clients, and Command actions that call back into solx-server. The app
        itself never needs it.
      </span>
      <div className="config-grid">
        <Field label="Address" view={view} keys={["server_host"]} hint="An IP address. 0.0.0.0 serves every interface.">
          <TextInput
            value={draft.host}
            placeholder={`${view.effective.server_host} (default)`}
            mono
            onChange={(host) => setDraft({ ...draft, host })}
          />
        </Field>
        <Field label="Port" view={view} keys={["server_port"]}>
          <TextInput
            type="number"
            value={draft.port}
            placeholder={`${view.effective.server_port} (default)`}
            onChange={(port) => setDraft({ ...draft, port })}
          />
        </Field>
      </div>
      <span className="dialog__label">
        Encryption (HTTPS) <RestartTag view={view} keys={["server_tls"]} />
      </span>
      <div className="config-rows">
        {(
          [
            [
              "default",
              "Default",
              IS_WINDOWS
                ? "On when the address above is a network address, with certificates this app makes; off on this machine only."
                : "Off. Choose On to have this app make certificates.",
            ],
            ["auto", "On", "HTTPS for other devices, with certificates this app makes and renews (see \"Secure connections\")."],
            ["off", "Off", "Plain HTTP only."],
            ...(draft.tls === "custom"
              ? ([["custom", "Your own certificate", "Set in Raw JSON as cert_file/key_file or store_thumbprint."]] as const)
              : []),
          ] as const
        ).map(([value, label, hint]) => (
          <label className="config-check" key={value}>
            <input
              type="radio"
              name="server-tls"
              checked={draft.tls === value}
              onChange={() => setDraft({ ...draft, tls: value })}
            />
            <span>
              {label} <span className="dialog__hint">{hint}</span>
            </span>
          </label>
        ))}
      </div>
      {!isLoopbackHost(draft.host) && tlsOff(draft) && (
        <div className="config-warning">
          Reachable from the network over unencrypted HTTP: bearer tokens travel in cleartext. Turn encryption on,
          or keep this server on a private network (e.g. Tailscale/WireGuard). Other devices connect with a code
          from "Connect another device"; this machine's root token is refused from the network.
        </div>
      )}
      {!isLoopbackHost(draft.host) && !tlsOff(draft) && (
        <span className="dialog__hint">
          Other devices use https://. Programs on this machine can keep using http://127.0.0.1. Other devices
          connect with a code from "Connect another device"; this machine's root token is refused from the network.
        </span>
      )}
      <Field
        label="Public address"
        view={view}
        keys={["server_public_url"]}
        hint="What other devices should use to reach this server, when it isn't the address above: a 0.0.0.0 bind, a Tailscale name or IP. Used in connect links, and added to the server certificate. An IP or host name gets this server's port (and https:// when encryption is on); a full URL is used as is."
      >
        <TextInput
          value={draft.publicUrl}
          placeholder="my-laptop.tailnet.ts.net"
          mono
          onChange={(publicUrl) => setDraft({ ...draft, publicUrl })}
        />
      </Field>
      <Field label="Client token lifetime (days)" view={view} keys={["client_token_ttl_days"]}>
        <TextInput
          type="number"
          value={draft.tokenDays}
          placeholder={`${view.effective.client_token_ttl_days} (default)`}
          onChange={(tokenDays) => setDraft({ ...draft, tokenDays })}
        />
      </Field>
      <Field
        label="Allowed browser origins"
        view={view}
        keys={["server_cors_origins"]}
        hint="Only used off loopback (on loopback any origin may call). Empty means no browser access."
      >
        <ChipTagInput
          chips={draft.origins}
          onChange={(origins) => setDraft({ ...draft, origins })}
          placeholder="https://solx.example.com"
        />
      </Field>
      <span className="dialog__hint">
        Clients on this machine use the root token, kept in its credential manager (
        <code>solx token show</code>). Other devices get a token of their own from a connect code.
      </span>
    </>
  );
}

function tlsChoice(value: SolxConfigFile["server_tls"]): TlsChoice {
  if (value == null) return "default";
  if (typeof value === "object") return "custom";
  return value === "auto" ? "auto" : "off";
}

/** Whether the draft serves plain HTTP to the network once the host restarts. */
function tlsOff(draft: ServerDraft): boolean {
  return draft.tls === "off" || (draft.tls === "default" && !IS_WINDOWS);
}

const server: SectionDef<ServerDraft> = {
  id: "server",
  title: "HTTP server",
  description: "The embedded solx-server that shares this app's data with other clients.",
  fromConfig: (raw) => ({
    embedded: raw.server_embedded ?? true,
    tls: tlsChoice(raw.server_tls),
    tlsCustom: typeof raw.server_tls === "object" ? raw.server_tls : null,
    host: raw.server_host ?? "",
    port: raw.server_port == null ? "" : String(raw.server_port),
    origins: raw.server_cors_origins ?? [],
    publicUrl: raw.server_public_url ?? "",
    tokenDays: raw.client_token_ttl_days == null ? "" : String(raw.client_token_ttl_days),
  }),
  toConfig: (draft) => ({
    // Stored only when off, so the default (on) stays implicit.
    server_embedded: draft.embedded ? undefined : false,
    server_tls: draft.tls === "default" ? undefined : draft.tls === "custom" ? (draft.tlsCustom ?? undefined) : draft.tls,
    server_host: draft.host.trim() || undefined,
    server_port: parseNumberInput(draft.port),
    server_cors_origins: nonEmpty(draft.origins),
    server_public_url: draft.publicUrl.trim() || undefined,
    client_token_ttl_days: parseNumberInput(draft.tokenDays),
  }),
  Editor: ServerEditor,
};

// ── secure connections ──────────────────────────────────────────────────

function formatExpiry(iso: string): string {
  const days = Math.floor((Date.parse(iso) - Date.now()) / 86_400_000);
  const when = new Date(iso).toLocaleDateString();
  return days < 0 ? `expired ${when}` : `expires ${when} (in ${days} days)`;
}

function keyText(cert: TlsCert): string {
  if (!cert.keyPresent) return "its key is missing";
  const where = cert.keyHardware ? "key on the TPM" : `key in ${cert.keyLocation}`;
  if (cert.keyExportable === false) return `${where}, can't be exported`;
  if (cert.keyExportable === true) return `${where}, exportable`;
  return where;
}

function SecureView({ view }: SectionEditorProps<null>) {
  const [status, setStatus] = useState<TlsStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [restart, setRestart] = useState(false);
  const [confirmReset, setConfirmReset] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await getTauri().config.tlsStatus());
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  // Again whenever the config is saved: the mode may have changed.
  useEffect(() => {
    void refresh();
  }, [refresh, view]);

  const run = async (what: string, action: () => Promise<TlsResult>) => {
    setBusy(what);
    setError(null);
    try {
      const result = await action();
      setStatus(result.status);
      if (result.restartRequired) setRestart(true);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
      setConfirmReset(false);
    }
  };

  const reveal = async () => {
    try {
      await revealCertsFolder();
    } catch (e) {
      setError(errorText(e));
    }
  };

  if (!status) {
    return error ? <div className="dialog__error">{error}</div> : <span className="dialog__hint">Loading…</span>;
  }
  const { ca } = status;
  const tlsOn = status.mode !== "off";

  return (
    <>
      {status.serverError ? (
        <div className="config-warning">The HTTP server didn't start: {status.serverError}</div>
      ) : status.servingHttps === true ? (
        <span className="dialog__hint">
          The HTTP server serves https:// to other devices. Programs on this machine can also use http://127.0.0.1.
        </span>
      ) : status.servingHttps === false ? (
        <span className="dialog__hint">
          The HTTP server serves plain HTTP{tlsOn ? "; restart the app to turn encryption on" : ""}.
        </span>
      ) : (
        <span className="dialog__hint">The HTTP server isn't running (see "HTTP server").</span>
      )}
      {status.modeError && <div className="config-warning">server_tls: {status.modeError}</div>}
      {(status.mode === "files" || status.mode === "store") && (
        <span className="dialog__hint">
          The server uses your own certificate (<code>server_tls</code> in Raw JSON). The certificates below are only
          served with encryption set to On.
        </span>
      )}
      {status.caError && <div className="config-warning">{status.caError}</div>}

      {!ca ? (
        <span className="dialog__hint">
          No certificates yet. With encryption on, the app creates them in <code>{status.dir}</code> when it starts.
        </span>
      ) : (
        <div className="config-card">
          <span className="dialog__label">Fingerprint</span>
          <span className="config-tls-fingerprint">{ca.fingerprint}</span>
          <span className="dialog__hint">
            Identifies this server's certificate authority. Another device can compare it with what it sees when it
            connects.
          </span>
          <span className="dialog__hint">
            Certificate authority: {formatExpiry(ca.root.notAfter)}, {keyText(ca.root)}.
          </span>
          {ca.server ? (
            <span className="dialog__hint">
              Server certificate: {formatExpiry(ca.server.notAfter)}, {keyText(ca.server)}. Renewed automatically when
              the app starts. Names:{" "}
              {ca.server.sans.map((name) => (
                <span className="config-tag config-tag--muted config-mono" key={name}>
                  {name}
                </span>
              ))}
            </span>
          ) : (
            <span className="dialog__hint">No server certificate yet; the app issues one when it starts.</span>
          )}
          <span className="dialog__hint">
            This machine{" "}
            {ca.trusted === true
              ? "trusts this certificate authority, so browsers here accept the server."
              : ca.trusted === false
                ? "doesn't trust this certificate authority. Browsers here warn about the server until you trust it."
                : "may or may not trust this certificate authority."}
          </span>
          <div className="dialog__input-row config-confirm">
            <button type="button" className="dialog__btn" onClick={() => void reveal()}>
              Open certificates folder
            </button>
            {ca.trusted === true ? (
              <button
                type="button"
                className="dialog__btn"
                disabled={busy !== null}
                onClick={() => void run("untrust", () => getTauri().config.tlsUntrust())}
              >
                {busy === "untrust" ? "Waiting for Windows…" : "Stop trusting on this machine"}
              </button>
            ) : (
              <button
                type="button"
                className="dialog__btn"
                disabled={busy !== null}
                onClick={() => void run("trust", () => getTauri().config.tlsTrust())}
              >
                {busy === "trust" ? "Waiting for Windows…" : "Trust on this machine"}
              </button>
            )}
            <button
              type="button"
              className="dialog__btn"
              disabled={busy !== null}
              onClick={() => void run("reissue", () => getTauri().config.tlsReissue())}
            >
              {busy === "reissue" ? "Reissuing…" : "Reissue server certificate"}
            </button>
            <button
              type="button"
              className="dialog__btn"
              disabled={busy !== null}
              onClick={() => setConfirmReset(true)}
            >
              Reset…
            </button>
          </div>
          {confirmReset && (
            <div className="config-warning">
              Delete this certificate authority and its keys? Every device that trusts it, and every solx client
              connected over https://, has to be set up again.
              {tlsOn ? " The app creates a new one when it restarts." : ""}
              <div className="dialog__input-row config-confirm">
                <button type="button" className="dialog__btn" disabled={busy !== null} onClick={() => setConfirmReset(false)}>
                  Cancel
                </button>
                <button
                  type="button"
                  className="dialog__btn dialog__btn--primary"
                  disabled={busy !== null}
                  onClick={() => void run("reset", () => getTauri().config.tlsReset())}
                >
                  Delete
                </button>
              </div>
            </div>
          )}
        </div>
      )}
      {restart && (
        <div className="config-warning">
          Restart the app to serve the change. Until then the HTTP server keeps its current certificate.
          <div className="dialog__input-row config-confirm">
            <button type="button" className="dialog__btn dialog__btn--primary" onClick={() => void restartApp()}>
              Restart now
            </button>
          </div>
        </div>
      )}
      {error && <div className="dialog__error">{error}</div>}
      <span className="dialog__hint">
        To use this server from a browser on another device, import <code>solx-root.crt</code> from the certificates
        folder into that device's trust store (Windows: <code>certutil -user -addstore Root solx-root.crt</code>).
      </span>
    </>
  );
}

const secure: SectionDef<null> = {
  id: "secure",
  title: "Secure connections",
  description: (
    <>
      The certificates behind HTTPS (encryption "On" in "HTTP server"): a certificate authority on this machine, and the
      server certificate it signs. Other devices trust the authority, so the server certificate can be renewed without
      them noticing.
    </>
  ),
  fromConfig: () => null,
  toConfig: () => ({}),
  Editor: SecureView,
  readOnly: true,
};

// ── storage ─────────────────────────────────────────────────────────────

const STORAGE = [
  ["data_directory", "Database directory"],
  ["files_directory", "Files directory"],
  ["packages_directory", "Packages directory"],
  ["docs_db", "Documents database file"],
  ["actions_db", "Actions database file"],
  ["types_db", "Types database file"],
] as const;

type StorageKey = (typeof STORAGE)[number][0];
type StorageDraft = Record<StorageKey, string>;

function StorageEditor({ draft, setDraft, view }: SectionEditorProps<StorageDraft>) {
  return (
    <>
      <div className="config-warning">
        Changing these points solx at a different location after a restart. Existing data is <strong>not</strong>{" "}
        moved: copy it yourself first, or solx starts empty.
      </div>
      {STORAGE.map(([key, label]) => (
        <Field key={key} label={label} view={view} keys={[key]}>
          <TextInput
            value={draft[key]}
            placeholder={`${view.effective[key] ?? ""} (default)`}
            mono
            onChange={(v) => setDraft({ ...draft, [key]: v })}
          />
        </Field>
      ))}
      <span className="dialog__hint">
        Config file: <code>{view.path}</code>
      </span>
    </>
  );
}

const storage: SectionDef<StorageDraft> = {
  id: "storage",
  title: "Storage",
  description: "Where solx keeps its databases and files.",
  fromConfig: (raw) => Object.fromEntries(STORAGE.map(([key]) => [key, raw[key] ?? ""])) as StorageDraft,
  toConfig: (draft) => Object.fromEntries(STORAGE.map(([key]) => [key, draft[key].trim() || undefined])),
  Editor: StorageEditor,
};

// ── packages ────────────────────────────────────────────────────────────
//
// Not a draft: installing and uninstalling act on the host immediately (they
// run the package's scripts), so this section has no Save/Revert and hands
// each resulting config back through `onView`.

function PackagesView({ view, onView }: SectionEditorProps<null>) {
  const pkgs = view.raw.installed_packages ?? [];
  const fileInput = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [warnings, setWarnings] = useState<string[]>([]);

  const run = async (label: string, job: () => Promise<PackageResult>) => {
    setBusy(label);
    setError(null);
    setWarnings([]);
    try {
      const result = await job();
      onView(result.config);
      setWarnings(result.warnings);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
      setConfirming(null);
    }
  };

  const install = (file: File) =>
    run(`Installing ${file.name}…`, async () =>
      getTauri().config.installPackage(new Uint8Array(await file.arrayBuffer())),
    );

  return (
    <>
      <div className="dialog__input-row">
        <input
          ref={fileInput}
          type="file"
          accept=".zip,application/zip"
          hidden
          onChange={(e) => {
            const file = e.target.files?.[0];
            e.target.value = ""; // so picking the same zip again still fires
            if (file) void install(file);
          }}
        />
        <button
          type="button"
          className="dialog__btn"
          disabled={busy !== null}
          onClick={() => fileInput.current?.click()}
          title="Install a package from a .zip"
        >
          ⬆ Install package…
        </button>
        {busy && <span className="dialog__hint">{busy}</span>}
      </div>
      <span className="dialog__hint">
        Unpacked into <code>{view.effective.packages_directory}</code>. Installing runs the package's install
        script and grants whatever commands and URLs it declares.
      </span>
      {error && <div className="dialog__error">{error}</div>}
      {warnings.map((w) => (
        <div className="config-warning" key={w}>
          {w}
        </div>
      ))}

      {!pkgs.length ? (
        <span className="dialog__hint">No packages installed.</span>
      ) : (
        <div className="config-rows">
          {pkgs.map((pkg) => (
            <div className="config-card config-card--package" key={pkg.name}>
              <button
                type="button"
                className="config-icon-btn config-card__close"
                disabled={busy !== null}
                aria-label={`Uninstall ${pkg.name}`}
                title="Uninstall"
                onClick={() => setConfirming(pkg.name)}
              >
                ✕
              </button>
              <div className="config-card__title">
                {pkg.name} <span className="config-tag config-tag--muted">{pkg.version || "?"}</span>
              </div>
              <span className="dialog__hint">
                Installed {pkg.installed_at || "?"} from <code>{pkg.path}</code>
              </span>
              {(pkg.granted_commands ?? []).length > 0 && (
                <span className="dialog__hint">Commands: {pkg.granted_commands.join(", ")}</span>
              )}
              {(pkg.granted_base_urls ?? []).length > 0 && (
                <span className="dialog__hint">URLs: {pkg.granted_base_urls.join(", ")}</span>
              )}
              {confirming === pkg.name && (
                <div className="config-warning">
                  Uninstall <strong>{pkg.name}</strong>? Its uninstall script runs and the commands and URLs it
                  was granted are revoked. If solx unpacked it from a zip, its files are deleted too. This can't
                  be undone.
                  <div className="dialog__input-row config-confirm">
                    <button
                      type="button"
                      className="dialog__btn"
                      disabled={busy !== null}
                      onClick={() => setConfirming(null)}
                    >
                      Cancel
                    </button>
                    <button
                      type="button"
                      className="dialog__btn dialog__btn--primary"
                      disabled={busy !== null}
                      onClick={() =>
                        void run(`Uninstalling ${pkg.name}…`, () => getTauri().config.uninstallPackage(pkg.name))
                      }
                    >
                      Uninstall
                    </button>
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </>
  );
}

const packages: SectionDef<null> = {
  id: "packages",
  title: "Packages",
  description: (
    <>
      Installed packages and what they were granted. Install from a package zip here, or with the solx CLI (
      <code>solx install-package</code>); uninstalling revokes the grants.
    </>
  ),
  fromConfig: () => null,
  toConfig: () => ({}),
  Editor: PackagesView,
  readOnly: true,
};

// ── connect another device ──────────────────────────────────────────────
//
// Not a draft either: codes and tokens live in the host's token registry,
// reached through `auth_exec` (the `solx:auth` permission, which only this
// window holds).

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function formatCountdown(secs: number): string {
  return `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, "0")}`;
}

function formatDate(iso: string | null): string {
  return iso ? new Date(iso).toLocaleString() : "never";
}

/** Remembers the last "Address for the link" in this window. */
const LINK_ADDRESS_KEY = "solx.connectLinkAddress";

const SCOPE_DATALIST_ID = "config-scope-suggestions";

/** Why a token's scopes don't hold yet, under `scope_enforcement`; `null` once they do. */
function enforcementNote(mode: ConfigView["effective"]["scope_enforcement"]): string | null {
  switch (mode) {
    case "enforce":
      return null;
    case "off":
      return 'Scopes aren\'t checked yet (scope_enforcement is "off"): every device can do everything.';
    default:
      return (
        'Scopes are only logged for now (scope_enforcement is "audit"): a device can still do everything, and ' +
        "solx scopes audit shows what it would be refused."
      );
  }
}

function DevicesView({ view }: SectionEditorProps<null>) {
  const [label, setLabel] = useState("");
  const [access, setAccess] = useState<ScopePreset | "custom">("full");
  const [customScopes, setCustomScopes] = useState<string[]>([]);
  const [linkAddress, setLinkAddress] = useState(() => {
    try {
      return localStorage.getItem(LINK_ADDRESS_KEY) ?? "";
    } catch {
      return "";
    }
  });
  const [code, setCode] = useState<ConnectCode | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [clients, setClients] = useState<ClientInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setClients(await getTauri().auth.listClients());
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const remaining = code ? Math.max(0, Math.round((Date.parse(code.expires_at) - now) / 1000)) : 0;

  // While a code is showing: count down, and watch for the device that
  // redeems it to appear in the list.
  useEffect(() => {
    if (!code) return;
    const tick = window.setInterval(() => setNow(Date.now()), 1000);
    const poll = window.setInterval(() => void refresh(), 4000);
    return () => {
      window.clearInterval(tick);
      window.clearInterval(poll);
    };
  }, [code, refresh]);

  useEffect(() => {
    if (code && remaining === 0) setCode(null);
  }, [code, remaining]);

  const chooseAccess = (next: ScopePreset | "custom") => {
    // Custom starts from the preset it replaces, to narrow or add to.
    if (next === "custom" && access !== "custom" && !customScopes.length) {
      setCustomScopes([...(PRESETS.find((p) => p.id === access)?.scopes ?? [])]);
    }
    setAccess(next);
  };

  const create = async () => {
    if (access === "custom" && !customScopes.length) {
      setError("Add at least one scope, or choose a preset.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const next = await getTauri().auth.createConnectCode({
        label: label.trim() || undefined,
        publicUrl: linkAddress.trim() || undefined,
        ...(access === "custom" ? { scopes: customScopes } : { preset: access }),
      });
      setNow(Date.now());
      setCode(next);
      setLabel("");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const revoke = async (id: string) => {
    setBusy(true);
    setError(null);
    try {
      await getTauri().auth.revokeClient(id);
      await refresh();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
      setConfirming(null);
    }
  };

  const copy = async (what: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(what);
      window.setTimeout(() => setCopied(null), 1500);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const loopbackOnly = isLoopbackHost(view.raw.server_host) && !view.raw.server_public_url;

  return (
    <>
      {!view.effective.server_embedded && (
        <div className="config-warning">
          The HTTP server is off (see "HTTP server"), so other devices have nothing to connect to. Codes made now
          still work once it's on, until they expire.
        </div>
      )}
      {view.effective.server_embedded && loopbackOnly && (
        <div className="config-warning">
          The HTTP server only listens on this machine ({view.effective.server_host}). To connect another device,
          set its address to 0.0.0.0 or this machine's network address in "HTTP server", then restart.
        </div>
      )}

      <div className="dialog__input-row">
        <input
          className="dialog__input"
          value={label}
          placeholder="Device name (optional), e.g. work laptop"
          spellCheck={false}
          onChange={(e) => setLabel(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !busy) void create();
          }}
        />
        <button
          type="button"
          className="dialog__btn dialog__btn--primary"
          disabled={busy}
          onClick={() => void create()}
        >
          {code ? "New code" : "Create connect code"}
        </button>
      </div>
      <div className="dialog__field">
        <span className="dialog__label">What the device may do</span>
        <div className="config-rows">
          {[...PRESETS, { id: "custom" as const, label: "Custom", hint: "Only the scopes listed below." }].map((p) => (
            <label className="config-check" key={p.id}>
              <input
                type="radio"
                name="device-access"
                checked={access === p.id}
                onChange={() => chooseAccess(p.id)}
              />
              <span>
                {p.label} <span className="dialog__hint">{p.hint}</span>
              </span>
            </label>
          ))}
        </div>
        {access === "custom" && (
          <>
            <ChipTagInput
              chips={customScopes}
              onChange={setCustomScopes}
              placeholder="read:document:/notes/*, exec:/packages/solx-google/*"
              datalistId={SCOPE_DATALIST_ID}
              datalistOptions={[...SCOPE_SUGGESTIONS]}
            />
            <span className="dialog__hint">
              <code>read:</code> or <code>write:</code> with <code>document</code>, <code>type</code>,{" "}
              <code>action</code> or <code>file</code>, optionally followed by <code>:/path/*</code>;{" "}
              <code>exec:/path/*</code> to run actions; <code>write:script</code> and the other kinds to save
              actions of that kind. The device's token gets exactly these, fixed when the code is made.
            </span>
          </>
        )}
        {enforcementNote(view.effective.scope_enforcement) && (
          <span className="dialog__hint">{enforcementNote(view.effective.scope_enforcement)}</span>
        )}
      </div>
      <Field
        label="Address for the link (optional)"
        view={view}
        hint={
          <>
            Use a different address than {view.raw.server_public_url ? "the public address" : "this machine's network address"},
            e.g. its Tailscale IP. An IP or host name gets this server's port; a full URL is used as is.
          </>
        }
      >
        <TextInput
          value={linkAddress}
          placeholder="100.101.102.103"
          mono
          onChange={(v) => {
            setLinkAddress(v);
            try {
              localStorage.setItem(LINK_ADDRESS_KEY, v);
            } catch {
              // Not remembered; it still applies to this code.
            }
          }}
        />
      </Field>

      {code && (
        <div className="config-connect-code" aria-live="polite">
          <span className="dialog__hint">On the other device, enter this code</span>
          <span className="config-connect-code__digits">{code.display_code}</span>
          {code.server_url && (
            <span className="dialog__hint">
              with the server address <code>{code.server_url}</code>{" "}
              <button
                type="button"
                className="dialog__btn"
                // With the pin, so a pasted address is pinned like the link.
                onClick={() =>
                  void copy("address", code.tls_pin ? `${code.server_url}#pin=${code.tls_pin}` : code.server_url!)
                }
              >
                {copied === "address" ? "Copied" : "Copy"}
              </button>
            </span>
          )}
          {code.url && (
            <span className="dialog__hint config-connect-code__link">
              or open <code>{code.url}</code>{" "}
              <button type="button" className="dialog__btn" onClick={() => void copy("link", code.url!)}>
                {copied === "link" ? "Copied" : "Copy link"}
              </button>
            </span>
          )}
          <span className="dialog__hint">Access: {describeScopes(code.scopes)}</span>
          <span className="dialog__hint">
            Expires in {formatCountdown(remaining)}. It works once, and anyone who has it can connect until then.
          </span>
        </div>
      )}
      {error && <div className="dialog__error">{error}</div>}

      <span className="dialog__label">Connected devices</span>
      {clients === null ? (
        <span className="dialog__hint">Loading…</span>
      ) : !clients.length ? (
        <span className="dialog__hint">No other devices have a token.</span>
      ) : (
        <div className="config-rows">
          {clients.map((c) => (
            <div className="config-card config-card--package" key={c.id}>
              <button
                type="button"
                className="config-icon-btn config-card__close"
                disabled={busy}
                aria-label={`Revoke ${c.label ?? c.id}`}
                title="Revoke"
                onClick={() => setConfirming(c.id)}
              >
                ✕
              </button>
              <div className="config-card__title">{c.label ?? "Unnamed device"}</div>
              <span className="dialog__hint">
                Connected {formatDate(c.created_at)} · last used {formatDate(c.last_used_at)} · expires{" "}
                {formatDate(c.expires_at)}
              </span>
              <span className="dialog__hint">Access: {describeScopes(c.scopes)}</span>
              {confirming === c.id && (
                <div className="config-warning">
                  Revoke this device's token? Its next request is refused, and it needs a new code to reconnect.
                  <div className="dialog__input-row config-confirm">
                    <button type="button" className="dialog__btn" disabled={busy} onClick={() => setConfirming(null)}>
                      Cancel
                    </button>
                    <button
                      type="button"
                      className="dialog__btn dialog__btn--primary"
                      disabled={busy}
                      onClick={() => void revoke(c.id)}
                    >
                      Revoke
                    </button>
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </>
  );
}

const devices: SectionDef<null> = {
  id: "devices",
  title: "Connect another device",
  description: (
    <>
      Give another computer, browser or MCP client a token of its own: create a code here and enter it there (or
      run <code>solx token connect-code</code> on this machine). Choose what the device may do before making the
      code; each device can be revoked on its own.
    </>
  ),
  fromConfig: () => null,
  toConfig: () => ({}),
  Editor: DevicesView,
  readOnly: true,
};

// ── raw ─────────────────────────────────────────────────────────────────

function RawEditor({ draft, setDraft }: SectionEditorProps<string>) {
  return <JsonEditField value={draft} onChange={setDraft} minLines={24} ariaLabel="solx-config.json" />;
}

const raw: SectionDef<string> = {
  id: "raw",
  title: "Raw JSON",
  description:
    "The whole file. Saving still goes through the same validation, so a value of the wrong type is rejected rather than written.",
  fromConfig: (cfg) => JSON.stringify(cfg, null, 2),
  toConfig: (text, current) => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch (e) {
      throw new Error(`Not valid JSON: ${e instanceof Error ? e.message : String(e)}`);
    }
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      throw new Error("solx-config.json must be a JSON object.");
    }
    const next = parsed as Record<string, unknown>;
    const keys = new Set([...Object.keys(current), ...Object.keys(next)]);
    return Object.fromEntries([...keys].map((k) => [k, next[k]]));
  },
  Editor: RawEditor,
};

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export const SECTIONS: SectionDef<any>[] = [
  allowlists,
  tools,
  exclusions,
  limits,
  environment,
  server,
  secure,
  devices,
  storage,
  packages,
  raw,
];
