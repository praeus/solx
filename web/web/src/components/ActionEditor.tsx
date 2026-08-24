/**
 * ActionEditor — modal editor for solx Actions.
 *
 * Mirrors the sol-browser ActionEditor at a reduced scope:
 *   - Core fields: name, path, caption, description, category, phrases,
 *     capabilities, parameter type, result type, action_type, exec artifact,
 *     invoke resource, raw action_config JSON.
 *   - Typed overlays for the two most common action types: Webhook headers +
 *     timeout_secs, Command cwd — layered onto action_config so the raw-JSON
 *     box stays an escape hatch instead of the only way to set them.
 *   - Attached files (`Action.files: FileRef[]`) via the shared `FilesField`,
 *     the same component/wire shape `DocumentEditor` uses.
 *   - Run: a single "Run Action" button (edit mode) that opens the detached
 *     `ActionRunner`.
 *
 * NOT in scope (no equivalent entity in solx): Permission/allowed-permission
 * fields, Schedule creation, custom-action project scaffolding (superseded
 * by the solx-quickjs package), run trace table, model instructions (legacy
 * sol-browser concept, not carried over).
 */

import { useEffect, useMemo, useState } from "react";
import type { ActionInputApp, ActionSummary } from "../api/api";
import { useCreateAction, useDocumentTypes } from "../api/hooks";
import { ActionRunner } from "./ActionRunner";
import { ChipTagInput } from "./ChipTagInput";
import { FilesField, coerceFilesField, type FileRef } from "./FilesField";
import { TypeSelector } from "./TypeSelector";

interface WebhookHeaderEntry {
  key: string;
  value: string;
}

function headersFromConfig(cfg: Record<string, unknown>): WebhookHeaderEntry[] {
  const headers = cfg.headers;
  if (!headers || typeof headers !== "object" || Array.isArray(headers)) return [];
  return Object.entries(headers as Record<string, unknown>).map(([key, value]) => ({
    key,
    value: String(value ?? ""),
  }));
}

const ACTION_TYPES = [
  { value: "", label: "WASM (default)" },
  { value: "wasm", label: "WASM" },
  { value: "webhook", label: "Webhook" },
  { value: "command", label: "Command" },
  { value: "internal", label: "Internal" },
];

interface Props {
  open: boolean;
  onClose: () => void;
  /** When provided, the editor is in edit mode; when null, create mode. */
  existing: { path: string; name: string } | null;
  /**
   * Optional preloaded entity (already fetched by the parent). When present,
   * the editor hydrates its fields from this snapshot.
   */
  existingEntity?: ActionSummary | null;
  /** Pre-fill values for create mode (e.g. default path from the current browse). */
  defaultPath?: string;
  onSaved?: (saved: { path: string; name: string }) => void;
}

export function ActionEditor({ open, onClose, existing, existingEntity, defaultPath, onSaved }: Props) {
  const isEdit = existing !== null;
  const typesQuery = useDocumentTypes();
  const createMutation = useCreateAction();

  const [path, setPath] = useState(defaultPath ?? "/");
  const [name, setName] = useState("");
  const [caption, setCaption] = useState("");
  const [description, setDescription] = useState("");
  const [category, setCategory] = useState("");
  const [phrases, setPhrases] = useState<string[]>([]);
  const [capabilities, setCapabilities] = useState<string[]>([]);
  // Full type references (e.g. "/types/core/Null"), not bare names.
  const [paramTypeRef, setParamTypeRef] = useState("");
  const [resultTypeRef, setResultTypeRef] = useState("");
  const [actionType, setActionType] = useState("");
  const [binName, setBinName] = useState("");
  const [fnName, setFnName] = useState("");
  const [actionConfigJson, setActionConfigJson] = useState("");

  // Typed overlays for the two most common action types — surfaced as real
  // fields instead of forcing every edit through the raw action_config JSON.
  const [webhookHeaders, setWebhookHeaders] = useState<WebhookHeaderEntry[]>([]);
  const [webhookTimeoutSecs, setWebhookTimeoutSecs] = useState("");
  const [commandCwd, setCommandCwd] = useState("");

  // Files attached to this action (e.g. the wasm/script bin_name points at
  // one of these). Same wire shape and component as DocumentEditor.
  const [attachedFiles, setAttachedFiles] = useState<FileRef[]>([]);

  // Run panel — a single button that opens the detached ActionRunner.
  const [runnerOpen, setRunnerOpen] = useState(false);

  const [error, setError] = useState<string | null>(null);

  // Reset when dialog opens.
  useEffect(() => {
    if (!open) return;
    setError(null);
    setRunnerOpen(false);
    if (existing && existingEntity) {
      // Edit mode — hydrate from the preloaded row.
      setPath(existingEntity.path);
      setName(existingEntity.name);
      setCaption(String(existingEntity.caption ?? ""));
      setDescription(String(existingEntity.description ?? ""));
      setCategory(String(existingEntity.category ?? ""));
      setPhrases(Array.isArray(existingEntity.phrases) ? existingEntity.phrases.map(String) : []);
      setCapabilities(
        Array.isArray(existingEntity.capabilities) ? existingEntity.capabilities.map(String) : [],
      );
      setParamTypeRef(String(existingEntity.paramTypeRef ?? ""));
      setResultTypeRef(String(existingEntity.resultTypeRef ?? ""));
      setActionType(String(existingEntity.actionType ?? "").toLowerCase());
      setBinName(String(existingEntity.binName ?? ""));
      setFnName(String(existingEntity.fnName ?? ""));
      const cfg = (existingEntity.actionConfig ?? {}) as Record<string, unknown>;
      setActionConfigJson("");
      setWebhookHeaders(headersFromConfig(cfg));
      setWebhookTimeoutSecs(cfg.timeout_secs != null ? String(cfg.timeout_secs) : "");
      setCommandCwd(typeof cfg.cwd === "string" ? cfg.cwd : "");
      setAttachedFiles(coerceFilesField(existingEntity.files));
    } else if (existing) {
      // Edit mode without a preloaded entity — fall back to id-only.
      setPath(existing.path);
      setName(existing.name);
      setWebhookHeaders([]);
      setWebhookTimeoutSecs("");
      setCommandCwd("");
      setAttachedFiles([]);
    } else {
      // Create mode.
      setPath(defaultPath ?? "/");
      setName("");
      setCaption("");
      setDescription("");
      setCategory("");
      setPhrases([]);
      setCapabilities([]);
      setParamTypeRef("");
      setResultTypeRef("");
      setActionType("");
      setBinName("");
      setFnName("");
      setActionConfigJson("");
      setWebhookHeaders([]);
      setWebhookTimeoutSecs("");
      setCommandCwd("");
      setAttachedFiles([]);
    }
  }, [open, existing, existingEntity, defaultPath]);

  const types = useMemo(
    () => (typesQuery.data?.items ?? []).slice().sort((a, b) => String(a.name).localeCompare(String(b.name))),
    [typesQuery.data],
  );

  if (!open) return null;

  const handleSave = async () => {
    setError(null);
    const trimmedPath = path.trim();
    const trimmedName = name.trim();
    if (!trimmedPath) {
      setError("Path is required.");
      return;
    }
    if (!trimmedName) {
      setError("Name is required.");
      return;
    }
    if (!trimmedPath.startsWith("/")) {
      setError("Path must start with '/'.");
      return;
    }

    // Best-effort: parse the raw action_config JSON if the user typed any.
    let actionConfig: Record<string, unknown> | undefined;
    if (actionConfigJson.trim()) {
      try {
        actionConfig = JSON.parse(actionConfigJson);
      } catch (e) {
        setError(`Raw action_config JSON is invalid: ${(e as Error).message}`);
        return;
      }
    }

    // Typed Webhook/Command overlays win over whatever the raw JSON had for
    // the same keys — the typed fields are what the user just edited.
    if (actionType === "webhook") {
      const headers: Record<string, string> = {};
      for (const { key, value } of webhookHeaders) {
        if (key.trim()) headers[key.trim()] = value;
      }
      const trimmedTimeout = webhookTimeoutSecs.trim();
      const timeout = trimmedTimeout ? Number(trimmedTimeout) : NaN;
      actionConfig = { ...(actionConfig ?? {}) };
      if (Object.keys(headers).length > 0) actionConfig.headers = headers;
      else delete actionConfig.headers;
      if (trimmedTimeout && Number.isFinite(timeout) && timeout > 0) {
        actionConfig.timeout_secs = timeout;
      } else {
        delete actionConfig.timeout_secs;
      }
    }
    if (actionType === "command") {
      const trimmedCwd = commandCwd.trim();
      if (trimmedCwd) {
        actionConfig = { ...(actionConfig ?? {}), cwd: trimmedCwd };
      } else if (actionConfig) {
        delete actionConfig.cwd;
      }
    }
    if (actionConfig && Object.keys(actionConfig).length === 0) {
      actionConfig = undefined;
    }

    try {
      const result = await createMutation.mutateAsync({
        path: trimmedPath,
        name: trimmedName,
        caption: caption.trim() || undefined,
        description: description.trim() || undefined,
        category: category.trim() || undefined,
        phrases: phrases.length ? phrases : undefined,
        capabilities: capabilities.length ? capabilities : undefined,
        paramTypeRef: paramTypeRef || undefined,
        resultTypeRef: resultTypeRef || undefined,
        actionType: (actionType || undefined) as ActionInputApp["actionType"],
        binName: binName.trim() || undefined,
        fnName: fnName.trim() || undefined,
        actionConfig: actionConfig,
        files: attachedFiles,
      });
      onSaved?.({ path: result.path, name: result.name });
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleRun = async () => {
    if (!existing) {
      setError("Save the action first.");
      return;
    }
    setRunnerOpen(true);
  };

  return (
    <>
      <div className="dialog-backdrop" onClick={onClose} />
      <div className="dialog dialog--wide" role="dialog" aria-label="Action editor">
        <div className="dialog__header">
          <h2 className="dialog__title">{isEdit ? "Edit Action" : "New Action"}</h2>
          <button className="dialog__close" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>

        <div className="dialog__body">
          <div className="form-field">
            <label className="form-label">
              Name <span className="required-star">*</span>
            </label>
            <input
              className="form-input"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="my-action"
              spellCheck={false}
              autoFocus
            />
          </div>

          <div className="form-field">
            <label className="form-label">
              Path <span className="required-star">*</span>
            </label>
            <input
              className="form-input"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="/tools"
              spellCheck={false}
            />
          </div>

          {/* Run — only in edit mode; opens the detached ActionRunner. */}
          {isEdit && (
            <div style={{ marginTop: 12 }}>
              <button
                className="dialog__btn dialog__btn--primary"
                onClick={() => void handleRun()}
              >
                Run Action
              </button>
            </div>
          )}

          <div className="doc-editor-section-sep">Execution</div>

          <div className="form-field">
            <label className="form-label">Action Type</label>
            <select
              className="form-input"
              value={actionType}
              onChange={(e) => setActionType(e.target.value)}
            >
              {ACTION_TYPES.map((opt) => (
                <option key={opt.value} value={opt.value}>
                  {opt.label}
                </option>
              ))}
            </select>
          </div>

          {actionType === "webhook" && (
            <>
              <div className="form-field">
                <label className="form-label">Headers</label>
                <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 4 }}>
                  Sent with every request. Stored under <code>action_config.headers</code>.
                </div>
                <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                  {webhookHeaders.map((row, idx) => (
                    <div key={idx} style={{ display: "flex", gap: 6 }}>
                      <input
                        className="form-input"
                        style={{ flex: 1 }}
                        value={row.key}
                        onChange={(e) =>
                          setWebhookHeaders((prev) =>
                            prev.map((r, i) => (i === idx ? { ...r, key: e.target.value } : r)),
                          )
                        }
                        placeholder="Header-Name"
                        spellCheck={false}
                      />
                      <input
                        className="form-input"
                        style={{ flex: 1 }}
                        value={row.value}
                        onChange={(e) =>
                          setWebhookHeaders((prev) =>
                            prev.map((r, i) => (i === idx ? { ...r, value: e.target.value } : r)),
                          )
                        }
                        placeholder="value"
                        spellCheck={false}
                      />
                      <button
                        type="button"
                        className="dialog__btn"
                        onClick={() => setWebhookHeaders((prev) => prev.filter((_, i) => i !== idx))}
                        aria-label="Remove header"
                        title="Remove header"
                      >
                        ×
                      </button>
                    </div>
                  ))}
                  <button
                    type="button"
                    className="dialog__btn"
                    style={{ alignSelf: "flex-start" }}
                    onClick={() => setWebhookHeaders((prev) => [...prev, { key: "", value: "" }])}
                  >
                    + Add header
                  </button>
                </div>
              </div>

              <div className="form-field">
                <label className="form-label">Timeout (seconds)</label>
                <input
                  className="form-input"
                  type="number"
                  min={1}
                  value={webhookTimeoutSecs}
                  onChange={(e) => setWebhookTimeoutSecs(e.target.value)}
                  placeholder="e.g. 30"
                />
              </div>
            </>
          )}

          {actionType === "command" && (
            <div className="form-field">
              <label className="form-label">Working Directory (cwd)</label>
              <input
                className="form-input"
                value={commandCwd}
                onChange={(e) => setCommandCwd(e.target.value)}
                placeholder="e.g. D:/Projects/solx-packages/my-package/bin"
                spellCheck={false}
              />
              <div style={{ fontSize: 11, color: "var(--text-muted)", marginTop: 2 }}>
                Stored under <code>action_config.cwd</code>. Overridden by the allowlist's own
                registered <code>cwd</code>, if one is set for this command's key.
              </div>
            </div>
          )}

          <div className="form-field">
            <label className="form-label">Exec Artifact (bin name)</label>
            <input
              className="form-input"
              value={binName}
              onChange={(e) => setBinName(e.target.value)}
              placeholder="e.g. my-actions.wasm"
              spellCheck={false}
              list="action-editor-bin-names"
            />
            <datalist id="action-editor-bin-names">
              {attachedFiles.map((f) => (
                <option key={f.relPath} value={f.name} />
              ))}
            </datalist>
          </div>

          <div className="form-field">
            <label className="form-label">Invoke Resource (fn name / URL / command)</label>
            <input
              className="form-input"
              value={fnName}
              onChange={(e) => setFnName(e.target.value)}
              placeholder="e.g. run, https://..., python3 my_script.py"
              spellCheck={false}
            />
          </div>

          <div className="doc-editor-section-sep">Types</div>

          <TypeSelector label="Parameter Type" value={paramTypeRef} onChange={setParamTypeRef} />

          <TypeSelector label="Result Type" value={resultTypeRef} onChange={setResultTypeRef} />

          <div className="doc-editor-section-sep">Metadata</div>

          <div className="form-field">
            <label className="form-label">Caption</label>
            <input
              className="form-input"
              value={caption}
              onChange={(e) => setCaption(e.target.value)}
              placeholder="Short display label"
            />
          </div>

          <div className="form-field">
            <label className="form-label">Description</label>
            <textarea
              className="form-textarea"
              rows={3}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Describe what this action does"
            />
          </div>

          <div className="form-field">
            <label className="form-label">Category</label>
            <input
              className="form-input"
              value={category}
              onChange={(e) => setCategory(e.target.value)}
              placeholder="e.g. extraction, search, filesystem"
              list="action-editor-categories"
            />
            <datalist id="action-editor-categories">
              {Array.from(new Set(types.map((t) => String(t.name)))).map((c) => (
                <option key={c} value={c} />
              ))}
            </datalist>
          </div>

          <div className="form-field">
            <label className="form-label">Phrases</label>
            <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 4 }}>
              Alternative names used for search matching
            </div>
            <ChipTagInput
              chips={phrases}
              onChange={setPhrases}
              placeholder="Type a phrase and press Enter"
            />
          </div>

          <div className="form-field">
            <label className="form-label">Capabilities</label>
            <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 4 }}>
              Tags used for search and filtering
            </div>
            <ChipTagInput
              chips={capabilities}
              onChange={setCapabilities}
              placeholder="search, wasm, extraction, hidden…"
            />
          </div>

          <details style={{ marginTop: 8 }}>
            <summary style={{ cursor: "pointer", fontSize: 12, color: "var(--text-muted)" }}>
              Advanced — attached files &amp; raw action_config JSON
            </summary>
            <div className="form-field" style={{ marginTop: 6 }}>
              <FilesField
                value={attachedFiles}
                onChange={setAttachedFiles}
                label="Attached files"
                uploadPrefix={`files/actions/${name.trim() || "shared"}/`}
              />
            </div>
            <div className="form-field" style={{ marginTop: 6 }}>
              <textarea
                className="form-textarea form-json"
                rows={6}
                value={actionConfigJson}
                onChange={(e) => setActionConfigJson(e.target.value)}
                spellCheck={false}
                placeholder='{"key": "value"}'
              />
            </div>
          </details>

          {error && <div className="dialog__error">{error}</div>}
        </div>

        <div className="dialog__footer">
          <button className="dialog__btn" onClick={onClose} disabled={createMutation.isPending}>
            Cancel
          </button>
          <button
            className="dialog__btn dialog__btn--primary"
            onClick={() => void handleSave()}
            disabled={createMutation.isPending}
          >
            {createMutation.isPending ? "Saving…" : "Save"}
          </button>
        </div>
      </div>

      {existing && existingEntity && (
        <ActionRunner
          open={runnerOpen}
          onClose={() => setRunnerOpen(false)}
          action={existingEntity}
        />
      )}
    </>
  );
}
