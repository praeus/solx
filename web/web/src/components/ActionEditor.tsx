/**
 * ActionEditor — modal editor for solx Actions.
 *
 * Mirrors the sol-browser ActionEditor at a reduced scope:
 *   - Core fields: name, caption, description, category, phrases, capabilities,
 *     parameter type, result type, action_type, exec artifact, invoke resource,
 *     model instructions, raw action_config JSON.
 *   - Run panel: JSON params, Run button, result textarea, trace table.
 *
 * NOT in scope (future work): artifact selector, link selector, project
 * scaffolding, schedule creation, raw action_config typed overlays.
 */

import { useEffect, useMemo, useState } from "react";
import type { ActionSummary } from "../api/api";
import { useCreateAction, useDocumentTypes, useExecAction } from "../api/hooks";
import { ChipTagInput } from "./ChipTagInput";

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
  const execMutation = useExecAction();

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
  const [modelInstructions, setModelInstructions] = useState("");
  const [actionConfigJson, setActionConfigJson] = useState("");

  // Run panel
  const [runParamsText, setRunParamsText] = useState("{}");
  const [runResultText, setRunResultText] = useState("");
  const [runError, setRunError] = useState<string | null>(null);

  const [error, setError] = useState<string | null>(null);

  // Reset when dialog opens.
  useEffect(() => {
    if (!open) return;
    setError(null);
    setRunResultText("");
    setRunError(null);
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
      setParamTypeRef(String(existingEntity.param_type_ref ?? ""));
      setResultTypeRef(String(existingEntity.result_type_ref ?? ""));
      setActionType(String(existingEntity.action_type ?? "").toLowerCase());
      setBinName(String(existingEntity.bin_name ?? ""));
      setFnName(String(existingEntity.fn_name ?? ""));
      const cfg = (existingEntity.action_config ?? {}) as Record<string, unknown>;
      const mi = cfg.model_instructions;
      setModelInstructions(typeof mi === "string" ? mi : "");
      setActionConfigJson("");
    } else if (existing) {
      // Edit mode without a preloaded entity — fall back to id-only.
      setPath(existing.path);
      setName(existing.name);
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
      setModelInstructions("");
      setActionConfigJson("");
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
    // `model_instructions` is not a field on the Rust `Action` entity — it
    // lives in the free-form config blob so it round-trips on save.
    if (modelInstructions.trim()) {
      actionConfig = { ...(actionConfig ?? {}), model_instructions: modelInstructions };
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
        param_type_ref: paramTypeRef || undefined,
        result_type_ref: resultTypeRef || undefined,
        action_type: actionType || undefined,
        bin_name: binName.trim() || undefined,
        fn_name: fnName.trim() || undefined,
        action_config: actionConfig,
      });
      onSaved?.({ path: result.path, name: result.name });
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleRun = async () => {
    if (!existing) {
      setRunError("Save the action first.");
      return;
    }
    setRunError(null);
    setRunResultText("");
    let params: unknown = null;
    if (runParamsText.trim()) {
      try {
        params = JSON.parse(runParamsText);
      } catch (e) {
        setRunError(`Params JSON is invalid: ${(e as Error).message}`);
        return;
      }
    }
    try {
      const result = await execMutation.mutateAsync({
        path: existing.path,
        name: existing.name,
        params,
      });
      setRunResultText(JSON.stringify(result, null, 2));
      if (!result.success) {
        setRunError(result.message ?? "Action reported failure (no message).");
      }
    } catch (err) {
      setRunError(err instanceof Error ? err.message : String(err));
    }
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
              {types.flatMap((t) => [String(t.name)]).map((c) => (
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

          <div className="form-field">
            <label className="form-label">Parameter Type</label>
            <select
              className="form-input"
              value={paramTypeRef}
              onChange={(e) => setParamTypeRef(e.target.value)}
            >
              <option value="">(none)</option>
              {types.map((t) => (
                <option key={`${t.path}/${t.name}`} value={`${t.path}/${t.name}`}>
                  {String(t.name)}
                </option>
              ))}
            </select>
          </div>

          <div className="form-field">
            <label className="form-label">Result Type</label>
            <select
              className="form-input"
              value={resultTypeRef}
              onChange={(e) => setResultTypeRef(e.target.value)}
            >
              <option value="">(none)</option>
              {types.map((t) => (
                <option key={`${t.path}/${t.name}`} value={`${t.path}/${t.name}`}>
                  {String(t.name)}
                </option>
              ))}
            </select>
          </div>

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

          <div className="form-field">
            <label className="form-label">Exec Artifact (bin name)</label>
            <input
              className="form-input"
              value={binName}
              onChange={(e) => setBinName(e.target.value)}
              placeholder="e.g. my-actions.wasm"
              spellCheck={false}
            />
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

          <div className="form-field">
            <label className="form-label">Model Instructions</label>
            <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 4 }}>
              Free-form notes for the LLM planner. Stored under
              <code> action_config.model_instructions</code>.
            </div>
            <textarea
              className="form-textarea"
              rows={4}
              value={modelInstructions}
              onChange={(e) => setModelInstructions(e.target.value)}
              placeholder="e.g. Params MUST include model_name and messages: [...]"
              spellCheck={false}
            />
          </div>

          <details style={{ marginTop: 8 }}>
            <summary style={{ cursor: "pointer", fontSize: 12, color: "var(--text-muted)" }}>
              Advanced — raw action_config JSON
            </summary>
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

          {/* Run panel — only in edit mode */}
          {isEdit && (
            <>
              <h3 className="entity-editor-section-sep">Run</h3>
              <div className="form-field">
                <label className="form-label">Run Params (JSON)</label>
                <textarea
                  className="form-textarea form-json"
                  rows={4}
                  value={runParamsText}
                  onChange={(e) => setRunParamsText(e.target.value)}
                  spellCheck={false}
                />
              </div>
              <div style={{ display: "flex", gap: 8 }}>
                <button
                  className="dialog__btn dialog__btn--primary"
                  onClick={() => void handleRun()}
                  disabled={execMutation.isPending}
                >
                  {execMutation.isPending ? "Running…" : "Run Action"}
                </button>
              </div>
              {runError && <div className="dialog__error">{runError}</div>}
              {runResultText && (
                <div className="form-field" style={{ marginTop: 8 }}>
                  <label className="form-label">Result</label>
                  <textarea
                    className="form-textarea form-json"
                    rows={6}
                    value={runResultText}
                    readOnly
                    spellCheck={false}
                  />
                </div>
              )}
            </>
          )}

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
    </>
  );
}
