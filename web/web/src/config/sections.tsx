/**
 * sections.tsx — one `SectionDef` per group of solx-config.json keys. Each
 * owns a draft shape convenient for its form and converts it back to file
 * keys; `ConfigApp` does the diffing, saving and restart prompting.
 *
 * Which keys need a restart comes from the host (`view.restartKeys`), not
 * from here — see `RESTART_KEYS` in solx-config.
 */
import { useMemo, type ReactNode } from "react";
import type { ConfigView, SolxConfigFile, ToolRule } from "@solx/tauri";
import { ChipTagInput } from "../components/ChipTagInput";
import { JsonEditField } from "../components/JsonEditField";
import { useActionPaths } from "../api/hooks";
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
  const pathsQuery = useActionPaths(500);
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
          with the <code>solx:hidden</code> tag, but can't unhide what's listed here.
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

interface ServerDraft {
  embedded: boolean;
  host: string;
  port: string;
  origins: string[];
}

function ServerEditor({ draft, setDraft, view }: SectionEditorProps<ServerDraft>) {
  const token = view.raw.server_token;
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
      {!isLoopbackHost(draft.host) && (
        <div className="config-warning">
          Reachable from the network: the token below is the only gate, and traffic is unencrypted HTTP. Use a
          private network (e.g. Tailscale/WireGuard) or a TLS reverse proxy.
        </div>
      )}
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
      <Field label="Token" view={view} hint="Bearer token for HTTP clients. Generated on first start.">
        <div className="dialog__input-row">
          <input className="dialog__input config-mono" type="text" readOnly value={token ?? "(not generated yet)"} />
          {token && (
            <button type="button" className="dialog__btn" onClick={() => void navigator.clipboard.writeText(token)}>
              Copy
            </button>
          )}
        </div>
      </Field>
    </>
  );
}

const server: SectionDef<ServerDraft> = {
  id: "server",
  title: "HTTP server",
  description: "The embedded solx-server that shares this app's data with other clients.",
  fromConfig: (raw) => ({
    embedded: raw.server_embedded ?? true,
    host: raw.server_host ?? "",
    port: raw.server_port == null ? "" : String(raw.server_port),
    origins: raw.server_cors_origins ?? [],
  }),
  toConfig: (draft) => ({
    // Stored only when off, so the default (on) stays implicit.
    server_embedded: draft.embedded ? undefined : false,
    server_host: draft.host.trim() || undefined,
    server_port: parseNumberInput(draft.port),
    server_cors_origins: nonEmpty(draft.origins),
  }),
  Editor: ServerEditor,
};

// ── storage ─────────────────────────────────────────────────────────────

const STORAGE = [
  ["data_directory", "Database directory"],
  ["files_directory", "Files directory"],
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

// ── packages (read-only) ────────────────────────────────────────────────

function PackagesView({ view }: SectionEditorProps<null>) {
  const pkgs = view.raw.installed_packages ?? [];
  if (!pkgs.length) return <span className="dialog__hint">No packages installed.</span>;
  return (
    <div className="config-rows">
      {pkgs.map((pkg) => (
        <div className="config-card" key={pkg.name}>
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
        </div>
      ))}
    </div>
  );
}

const packages: SectionDef<null> = {
  id: "packages",
  title: "Packages",
  description: (
    <>
      Installed packages and what they were granted. Install and uninstall with the solx CLI (
      <code>solx install-package</code>); uninstalling revokes the grants.
    </>
  ),
  fromConfig: () => null,
  toConfig: () => ({}),
  Editor: PackagesView,
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
export const SECTIONS: SectionDef<any>[] = [allowlists, tools, limits, environment, server, storage, packages, raw];
