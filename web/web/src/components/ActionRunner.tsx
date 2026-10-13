/**
 * ActionRunner — run an action detached, watch its console live, cancel it.
 *
 * Two-pane layout (mirrors DocumentEditor's side-by-side panes):
 *
 *   LEFT  — Params (top) + Result (bottom) + Start/Stop.
 *           Params has two tabs:
 *             • JSON   — a CodeMirror JSON editor pre-filled from the
 *               action's parameter type schema.
 *             • Fields — a schema-driven form (`FieldsEditorPanel`) built
 *               from the same parameter type.
 *           Result is a read-only CodeMirror JSON editor showing the
 *           invocation's result once it reaches a terminal state.
 *   RIGHT — Console (always shown) + a conditional Widget tab that appears
 *           when the action's `resultTypeRef` is
 *           `/builtin/types/WidgetDescriptor`. The Widget tab is currently a
 *           stub: it renders the descriptor's `tag_name` / `bin_name` /
 *           `fields`; mounting the bundle is not yet implemented.
 *
 * Instead of a plain "Run Action" (a single synchronous `exec`
 * request/response round trip), this drives the async invocation primitives
 * (`solx-actions/src/invocations.rs`, dispatched as ordinary Internal
 * actions under `/builtin/action/*` and `/builtin/console/*`):
 *
 *   1. `start`  → returns an `invocation_id` immediately; the action keeps
 *      running server-side.
 *   2. `poll`   → long-polled in a loop for status until it reaches a
 *      terminal state (ok / failed / cancelled / timeout / interrupted).
 *   3. `tail`   → long-polled in a parallel loop for console output,
 *      starting from the `console_seq_start` `start` returned so it never
 *      shows a previous run's entries.
 *   4. `stop`   → requests cancellation; cooperative first (the action
 *      notices via `action_cancelled` and exits), force-abortable on a
 *      second click once already `cancelling`.
 *
 * Closing the dialog only stops *watching* — it does not call `stop`. A
 * detached invocation is exactly that: it keeps running whether or not
 * anything is still polling it.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import {
  isTerminalInvocationStatus,
  pollInvocation,
  splitTypeRef,
  startInvocation,
  stopInvocation,
  tailConsole,
  type ActionSummary,
  type ConsoleEntry,
  type Invocation,
} from "../api/api";
import { useActions, useDocuments, useType } from "../api/hooks";
import { getConnection } from "../api/connection";
import { mountWidget, startDevBundleWatch, type WidgetDescriptor } from "@solx/widgets/host";
import type { JsonValue } from "@solx/surface";
import { FieldsEditorPanel } from "./FieldsEditorPanel";
import { JsonEditField } from "./JsonEditField";
import { defaultValueFromSchema } from "./schemaDefaults";
import {
  isNullableSchema,
  propertySchema,
  refKindFromSchema,
  requiredFieldNames,
  schemaValueType,
  valueFromUnknown,
  type RefFieldValue,
} from "./schemaFields";

interface Props {
  open: boolean;
  onClose: () => void;
  /** The full action entity — carries paramTypeRef/resultTypeRef for the
   *  params editor and widget detection, plus path/name for invocation. */
  action: ActionSummary;
}

const POLL_WAIT_SECS = 20;
const TAIL_WAIT_SECS = 20;

const WIDGET_DESCRIPTOR_REF = "/builtin/types/WidgetDescriptor";

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// ───────────────────────────────────────────────────────────────────────
// Last-used params persistence (localStorage, keyed per action).
// ───────────────────────────────────────────────────────────────────────

const STORAGE_PREFIX = "solx:action-runner:";

interface SavedParams {
  paramsMode: "json" | "fields";
  paramsJsonText: string;
  fieldValues: Record<string, string>;
  refValues: Record<string, RefFieldValue>;
}

function loadSavedParams(key: string): SavedParams | null {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<SavedParams>;
    if (!parsed || typeof parsed !== "object") return null;
    return {
      paramsMode: parsed.paramsMode === "json" ? "json" : "fields",
      paramsJsonText:
        typeof parsed.paramsJsonText === "string" ? parsed.paramsJsonText : "{}",
      fieldValues:
        parsed.fieldValues && typeof parsed.fieldValues === "object"
          ? parsed.fieldValues
          : {},
      refValues:
        parsed.refValues && typeof parsed.refValues === "object"
          ? parsed.refValues
          : {},
    };
  } catch {
    return null;
  }
}

function saveParams(key: string, value: SavedParams): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Ignore quota / privacy-mode errors.
  }
}

export function ActionRunner({ open, onClose, action }: Props) {
  // ── Params state (JSON tab + Fields tab) ──
  const [paramsMode, setParamsMode] = useState<"json" | "fields">("fields");
  const [paramsJsonText, setParamsJsonText] = useState("{}");
  const [fieldValues, setFieldValues] = useState<Record<string, string>>({});
  const [refValues, setRefValues] = useState<Record<string, RefFieldValue>>({});

  // ── Invocation state ──
  const [invocation, setInvocation] = useState<Invocation | null>(null);
  const [consoleEntries, setConsoleEntries] = useState<ConsoleEntry[]>([]);
  const [startError, setStartError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [stopping, setStopping] = useState(false);

  // ── Right-pane tab state ──
  const [rightTab, setRightTab] = useState<"result" | "console" | "widget">("result");

  // `stoppedRef` tears down both watch loops (dialog closed / unmounted).
  // `runningRef` tracks just the invocation's own liveness so the console
  // loop knows when to stop long-polling and drain to a final zero-wait read.
  const stoppedRef = useRef(false);
  const runningRef = useRef(false);
  const consoleBoxRef = useRef<HTMLDivElement | null>(null);
  // Persistence: `hasSavedRef` tells the schema-default pre-fill effect to
  // skip (we already restored saved values); `skipSaveRef` suppresses the
  // save effect for the single commit that hydrates state on open, so we
  // never write a previous action's stale values under this action's key.
  const hasSavedRef = useRef(false);
  const skipSaveRef = useRef(false);

  // Per-action localStorage key for the last-used params.
  const storageKey = useMemo(
    () => `${STORAGE_PREFIX}${action.path}/${action.name}`,
    [action.path, action.name],
  );

  // ── Resolve the parameter type (for the params editor) ──
  const paramRef = action.paramTypeRef;
  const paramSplit = useMemo(
    () => (paramRef ? splitTypeRef(paramRef) : null),
    [paramRef],
  );
  const paramTypeQuery = useType(
    paramSplit?.path,
    paramSplit?.name,
    { enabled: Boolean(paramSplit) },
  );
  const paramSchema = paramTypeQuery.data?.schema ?? null;

  // ── Entity names for DocRef/ActionRef datalists in the Fields tab ──
  const documentsListQuery = useDocuments({ limit: 500, sortBy: "name", sortOrder: "asc" });
  const actionsListQuery = useActions({ limit: 500, sortBy: "name", sortOrder: "asc" });
  const documentNames = useMemo(
    () =>
      (documentsListQuery.data?.items ?? []).map((d) => String(d.name ?? "")).filter(Boolean),
    [documentsListQuery.data],
  );
  const actionNames = useMemo(
    () =>
      (actionsListQuery.data?.items ?? []).map((a) => String(a.name ?? "")).filter(Boolean),
    [actionsListQuery.data],
  );

  // ── Reset for a fresh run whenever the dialog opens (or the target action
  //    changes while it's open). ──
  useEffect(() => {
    if (!open) return;
    skipSaveRef.current = true;
    const saved = loadSavedParams(storageKey);
    hasSavedRef.current = Boolean(saved);
    if (saved) {
      setParamsMode(saved.paramsMode);
      setParamsJsonText(saved.paramsJsonText);
      setFieldValues(saved.fieldValues);
      setRefValues(saved.refValues);
    } else {
      setParamsMode("fields");
      setParamsJsonText("{}");
      setFieldValues({});
      setRefValues({});
    }
    setInvocation(null);
    setConsoleEntries([]);
    setStartError(null);
    setStarting(false);
    setStopping(false);
    setRightTab("result");
  }, [open, action.path, action.name, storageKey]);

  // ── Pre-fill params from the parameter type schema once it loads ──
  useEffect(() => {
    if (!open || !paramSchema) return;
    // If we restored saved values on open, don't clobber them with defaults.
    if (hasSavedRef.current) return;
    const def = defaultValueFromSchema(paramSchema);
    const text = JSON.stringify(def, null, 2);
    setParamsJsonText(text);
    // Hydrate the Fields tab from the same default.
    const props = propertySchema(paramSchema);
    const nextFieldValues: Record<string, string> = {};
    const nextRefValues: Record<string, RefFieldValue> = {};
    for (const [fieldName, fieldSchema] of Object.entries(props)) {
      const refKind = refKindFromSchema(fieldSchema);
      if (refKind) {
        nextRefValues[fieldName] = { kind: refKind, mode: "name", value: "" };
        continue;
      }
      const fieldType = schemaValueType(fieldSchema);
      const source = (def as Record<string, unknown>)[fieldName];
      nextFieldValues[fieldName] = valueFromUnknown(source, fieldType);
    }
    setFieldValues(nextFieldValues);
    setRefValues(nextRefValues);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, paramSchema]);

  // ── Persist the last-used params whenever they change while open ──
  useEffect(() => {
    if (!open) return;
    if (skipSaveRef.current) {
      skipSaveRef.current = false;
      return;
    }
    saveParams(storageKey, { paramsMode, paramsJsonText, fieldValues, refValues });
  }, [open, storageKey, paramsMode, paramsJsonText, fieldValues, refValues]);

  // ── Build the params object from the Fields tab's current state. This is
  //    also what keeps the JSON tab in sync (see the sync effect below), so
  //    it's computed independently of `paramsMode`. ──
  //
  //    A blank scalar field maps to `null` only when the field's schema
  //    actually allows null. A blank *required* field falls back to the
  //    field's schema default so required-non-nullable fields (e.g. a plain
  //    `string` `redirect_uri`) never submit `null` and trip `"null is not
  //    of type \"string\""` validation. A blank *optional* field is omitted
  //    from the payload entirely, rather than defaulted -- an unset optional
  //    integer becoming a literal `0` (etc.) reads to the receiving action as
  //    a real, meaningful value rather than "not provided", which breaks any
  //    `if $params.x == null` fallback logic on the other end.
  const paramsFromFields = useMemo<unknown>(() => {
    if (!paramSchema) return null;
    const props = propertySchema(paramSchema);
    const required = requiredFieldNames(paramSchema);
    const contents: Record<string, unknown> = {};
    for (const [fieldName, fieldSchema] of Object.entries(props)) {
      const refKind = refKindFromSchema(fieldSchema);
      if (refKind) {
        const refValue = refValues[fieldName];
        if (!refValue || !refValue.value.trim()) {
          contents[fieldName] = null;
        } else {
          contents[fieldName] = { [refValue.mode]: refValue.value.trim() };
        }
        continue;
      }
      const raw = (fieldValues[fieldName] ?? "").trim();
      const fieldType = schemaValueType(fieldSchema);
      if (!raw) {
        if (fieldType === "boolean") {
          contents[fieldName] = false;
        } else if (isNullableSchema(fieldSchema)) {
          contents[fieldName] = null;
        } else if (required.has(fieldName)) {
          contents[fieldName] = defaultValueFromSchema(
            fieldSchema,
            paramSchema as Record<string, unknown>,
          );
        }
        // else: optional, non-nullable, left blank -- omit the key.
        continue;
      }
      if (fieldType === "number" || fieldType === "integer") {
        const n = Number(raw);
        contents[fieldName] = Number.isFinite(n)
          ? fieldType === "integer"
            ? Math.trunc(n)
            : n
          : null;
      } else if (fieldType === "boolean") {
        contents[fieldName] = raw.toLowerCase() === "true";
      } else if (fieldType === "array" || fieldType === "object" || fieldType === "rich_text") {
        try {
          contents[fieldName] = JSON.parse(raw);
        } catch {
          contents[fieldName] = null;
        }
      } else {
        contents[fieldName] = raw;
      }
    }
    return contents;
  }, [paramSchema, fieldValues, refValues]);

  // ── Build the params object from whichever tab is active ──
  const builtParams = useMemo<unknown>(() => {
    if (paramsMode === "json") {
      try {
        return JSON.parse(paramsJsonText);
      } catch {
        return null;
      }
    }
    return paramsFromFields;
  }, [paramsMode, paramsJsonText, paramsFromFields]);

  // ── Keep the JSON tab in sync while the Fields tab is being edited ──
  useEffect(() => {
    if (!open || !paramSchema) return;
    if (paramsMode !== "fields") return;
    const text = JSON.stringify(paramsFromFields ?? null, null, 2);
    setParamsJsonText((prev) => (prev === text ? prev : text));
  }, [open, paramsMode, paramSchema, paramsFromFields]);

  // ── Keep the Fields tab in sync while the JSON tab is being edited ──
  useEffect(() => {
    if (!open || !paramSchema) return;
    if (paramsMode !== "json") return;
    let parsed: unknown;
    try {
      parsed = JSON.parse(paramsJsonText);
    } catch {
      return;
    }
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return;
    const source = parsed as Record<string, unknown>;
    const props = propertySchema(paramSchema);
    const nextFieldValues: Record<string, string> = {};
    const nextRefValues: Record<string, RefFieldValue> = {};
    for (const [fieldName, fieldSchema] of Object.entries(props)) {
      const refKind = refKindFromSchema(fieldSchema);
      if (refKind) {
        const refSource = source[fieldName];
        if (refSource && typeof refSource === "object") {
          const refObj = refSource as Record<string, unknown>;
          if (typeof refObj.name === "string") {
            nextRefValues[fieldName] = { kind: refKind, mode: "name", value: refObj.name };
          } else if (typeof refObj.id === "string") {
            nextRefValues[fieldName] = { kind: refKind, mode: "id", value: refObj.id };
          } else {
            nextRefValues[fieldName] = { kind: refKind, mode: "name", value: "" };
          }
        } else {
          nextRefValues[fieldName] = { kind: refKind, mode: "name", value: "" };
        }
        continue;
      }
      const fieldType = schemaValueType(fieldSchema);
      nextFieldValues[fieldName] = valueFromUnknown(source[fieldName], fieldType);
    }
    setFieldValues(nextFieldValues);
    setRefValues(nextRefValues);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, paramsMode, paramSchema, paramsJsonText]);

  const paramsIsValidJson = useMemo(() => {
    if (paramsMode !== "json") return true;
    try {
      JSON.parse(paramsJsonText);
      return true;
    } catch {
      return false;
    }
  }, [paramsMode, paramsJsonText]);

  // ── Widget detection ──
  const isWidgetAction = action.resultTypeRef === WIDGET_DESCRIPTOR_REF;

  // Stop watching (the loops check `stoppedRef` after each await) on close
  // or unmount.
  useEffect(() => {
    if (open) return;
    stoppedRef.current = true;
  }, [open]);

  useEffect(() => {
    return () => {
      stoppedRef.current = true;
    };
  }, []);

  useEffect(() => {
    if (consoleBoxRef.current) {
      consoleBoxRef.current.scrollTop = consoleBoxRef.current.scrollHeight;
    }
  }, [consoleEntries]);

  const runPollLoop = async (invocationId: string) => {
    while (!stoppedRef.current) {
      try {
        const inv = await pollInvocation(invocationId, POLL_WAIT_SECS);
        if (stoppedRef.current) return;
        if (!inv) return;
        setInvocation(inv);
        if (isTerminalInvocationStatus(inv.status)) {
          runningRef.current = false;
          return;
        }
      } catch {
        if (stoppedRef.current) return;
        // Transient network hiccup — back off briefly and keep polling.
        await sleep(1500);
      }
    }
  };

  const runConsoleLoop = async (actionRef: string, seqStart: number) => {
    let cursor: number | null = seqStart;
    while (!stoppedRef.current) {
      const waitSecs = runningRef.current ? TAIL_WAIT_SECS : 0;
      try {
        const res = await tailConsole(actionRef, { cursor, limit: 500, waitSecs });
        if (stoppedRef.current) return;
        if (res.entries.length > 0) {
          setConsoleEntries((prev) => [...prev, ...res.entries]);
        }
        if (res.next_cursor != null) cursor = res.next_cursor;
        // Once the invocation is terminal, a zero-wait read that comes back
        // empty means we've drained everything — stop polling.
        if (!runningRef.current && res.entries.length === 0) return;
      } catch {
        if (stoppedRef.current) return;
        await sleep(1500);
      }
    }
  };

  const handleStart = async () => {
    setStartError(null);
    if (!paramsIsValidJson) {
      setStartError("Params JSON is invalid.");
      return;
    }
    setStarting(true);
    setInvocation(null);
    setConsoleEntries([]);
    stoppedRef.current = false;
    runningRef.current = true;
    try {
      const inv = await startInvocation(action.path, action.name, builtParams);
      setInvocation(inv);
      void runPollLoop(inv.invocation_id);
      void runConsoleLoop(inv.action_ref, inv.console_seq_start);
    } catch (err) {
      runningRef.current = false;
      setStartError(err instanceof Error ? err.message : String(err));
    } finally {
      setStarting(false);
    }
  };

  const handleStop = async () => {
    if (!invocation) return;
    setStopping(true);
    try {
      const force = invocation.status === "cancelling";
      const updated = await stopInvocation(invocation.invocation_id, { force });
      if (updated) setInvocation(updated);
    } catch (err) {
      setStartError(err instanceof Error ? err.message : String(err));
    } finally {
      setStopping(false);
    }
  };

  const status = invocation?.status ?? null;
  const terminal = status ? isTerminalInvocationStatus(status) : false;
  const canStart = starting || !invocation || terminal;
  const canStop = !!invocation && !terminal;

  const resultText = useMemo(() => {
    if (!invocation || !terminal) return "";
    if (invocation.error) return invocation.error;
    return JSON.stringify(invocation.result ?? null, null, 2);
  }, [invocation, terminal]);

  if (!open) return null;

  return (
    <>
      <div className="dialog-backdrop" onClick={onClose} />
      <div className="dialog dialog--xxl" role="dialog" aria-label="Run action">
        <div className="dialog__header">
          <h2 className="dialog__title">
            Run: {action.path}/{action.name}
          </h2>
          <button className="dialog__close" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>

        <div className="dialog__body">
          <div className="doc-editor-layout">
            {/* ── LEFT: params + result + start/stop ── */}
            <div className="doc-editor-pane">
              <div className="doc-editor-tabs">
                <button
                  type="button"
                  className={`doc-editor-tab${paramsMode === "fields" ? " doc-editor-tab--active" : ""}`}
                  onClick={() => setParamsMode("fields")}
                >
                  Fields
                </button>
                <button
                  type="button"
                  className={`doc-editor-tab${paramsMode === "json" ? " doc-editor-tab--active" : ""}`}
                  onClick={() => setParamsMode("json")}
                >
                  JSON
                </button>
              </div>

              {paramsMode === "json" ? (
                <JsonEditField
                  value={paramsJsonText}
                  onChange={setParamsJsonText}
                  placeholder={`{\n  "field": "value"\n}`}
                  ariaLabel="Action params as JSON"
                  minLines={10}
                />
              ) : (
                <FieldsEditorPanel
                  schema={paramSchema}
                  values={fieldValues}
                  refValues={refValues}
                  onValuesChange={setFieldValues}
                  onRefValuesChange={setRefValues}
                  documentNames={documentNames}
                  actionNames={actionNames}
                  emptyMessage={
                    paramSchema
                      ? "No fields for this parameter type."
                      : "No parameter type set — use JSON mode."
                  }
                />
              )}

              {startError && <div className="dialog__error">{startError}</div>}
            </div>

            {/* ── RIGHT: result + console + widget ── */}
            <div className="doc-editor-pane doc-editor-pane--preview">
              <div className="doc-editor-tabs">
                <button
                  type="button"
                  className={`doc-editor-tab${rightTab === "result" ? " doc-editor-tab--active" : ""}`}
                  onClick={() => setRightTab("result")}
                >
                  Result
                </button>
                <button
                  type="button"
                  className={`doc-editor-tab${rightTab === "console" ? " doc-editor-tab--active" : ""}`}
                  onClick={() => setRightTab("console")}
                >
                  Console
                </button>
                {isWidgetAction && (
                  <button
                    type="button"
                    className={`doc-editor-tab${rightTab === "widget" ? " doc-editor-tab--active" : ""}`}
                    onClick={() => setRightTab("widget")}
                  >
                    Widget
                  </button>
                )}
              </div>

              {rightTab === "result" ? (
                <JsonEditField
                  value={resultText}
                  onChange={() => {}}
                  readOnly
                  noLint
                  placeholder="Run the action to see its result."
                  ariaLabel="Action result"
                  minLines={8}
                />
              ) : rightTab === "console" ? (
                <div
                  ref={consoleBoxRef}
                  style={{
                    height: 360,
                    overflowY: "auto",
                    background: "var(--bg-sunken)",
                    border: "1px solid var(--border)",
                    borderRadius: "var(--radius)",
                    padding: 8,
                    fontFamily: "var(--font-mono)",
                    fontSize: 12,
                  }}
                >
                  {consoleEntries.length === 0 ? (
                    <div style={{ color: "var(--text-muted)" }}>No console output yet.</div>
                  ) : (
                    consoleEntries.map((entry) => (
                      <div key={entry.seq} style={{ display: "flex", gap: 6, whiteSpace: "pre-wrap" }}>
                        <span style={{ color: "var(--text-faint)", flexShrink: 0 }}>
                          {formatTime(entry.ts)}
                        </span>
                        <span style={{ color: levelColor(entry.level), flexShrink: 0 }}>
                          [{entry.level}]
                        </span>
                        <span>
                          {entry.message ??
                            (entry.data != null ? JSON.stringify(entry.data) : "")}
                        </span>
                      </div>
                    ))
                  )}
                </div>
              ) : (
                <WidgetPanel result={invocation?.result ?? null} />
              )}
            </div>
          </div>
        </div>

        <div className="dialog__footer">
          <div style={{ display: "flex", gap: 8, alignItems: "center", marginRight: "auto" }}>
            <button
              className="dialog__btn dialog__btn--primary"
              onClick={() => void handleStart()}
              disabled={!canStart}
            >
              {starting ? "Starting…" : invocation ? "Run again" : "Start"}
            </button>
            <button
              className="dialog__btn dialog__btn--danger"
              onClick={() => void handleStop()}
              disabled={!canStop || stopping}
            >
              {stopping ? "…" : status === "cancelling" ? "Force stop" : "Stop"}
            </button>
            {status && <StatusBadge status={status} />}
          </div>
          <button className="dialog__btn" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </>
  );
}

// ───────────────────────────────────────────────────────────────────────
// Widget panel — fetches the bundle, imports it, and mounts the descriptor's
// custom element via @solx/widgets' mountWidget (solx-core/docs/widget-actions.md §3).
// ───────────────────────────────────────────────────────────────────────

function isWidgetDescriptor(value: unknown): value is WidgetDescriptor {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as Record<string, unknown>).tag_name === "string" &&
    typeof (value as Record<string, unknown>).bin_name === "string"
  );
}

function WidgetPanel({ result }: { result: unknown }) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [mountError, setMountError] = useState<string | null>(null);

  const descriptor = isWidgetDescriptor(result) ? result : null;
  // Depend on content, not object identity — `result` is a fresh reference
  // on every poll tick even when the descriptor itself hasn't changed, and
  // remounting on every tick would be needless churn.
  const descriptorKey = descriptor
    ? `${descriptor.tag_name} ${descriptor.bin_name} ${JSON.stringify(descriptor.fields ?? null)}`
    : null;

  useEffect(() => {
    setMountError(null);
    const container = containerRef.current;
    if (!descriptor || !container) return;
    let cancelled = false;
    let stopDevWatch: (() => void) | null = null;
    const conn = getConnection();
    mountWidget(descriptor, container, {
      files: conn.files,
      client: {
        actions: {
          exec: (path, name, params) => conn.actions.exec(path, name, (params ?? {}) as JsonValue),
        },
        // Delegated to the same helpers the app drives its own detached runs
        // with, so the long-poll and cancellation behaviour a widget gets is
        // the behaviour ActionRunner itself is tested against.
        invocations: {
          start: (path, name, params) => startInvocation(path, name, params),
          poll: (invocationId, waitSecs) => pollInvocation(invocationId, waitSecs),
          stop: (invocationId, opts) => stopInvocation(invocationId, { force: opts?.force }),
          tailConsole: (actionRef, opts) => tailConsole(actionRef, opts),
        },
      },
    }).then(() => {
      // Dev-only: a widget bundle can't be hot-swapped in place (the tag's
      // customElements.define is permanent), so the only way to pick up a
      // rebuilt bundle is a full reload -- see solx-widgets/src/host/devReload.ts.
      if (!cancelled && import.meta.env.DEV) {
        stopDevWatch = startDevBundleWatch(conn.files, descriptor.bin_name);
      }
    }).catch((err: unknown) => {
      if (!cancelled) setMountError(err instanceof Error ? err.message : String(err));
    });
    return () => {
      cancelled = true;
      stopDevWatch?.();
      container.replaceChildren();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [descriptorKey]);

  if (!descriptor) {
    return (
      <div className="dialog__hint">
        No widget result yet — run the action to produce a widget descriptor.
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      <div ref={containerRef} style={{ minHeight: 120 }} />
      {mountError && (
        <div className="dialog__hint" style={{ color: "var(--danger, #c0392b)" }}>
          Failed to mount widget: {mountError}
        </div>
      )}
    </div>
  );
}

function formatTime(ts: string): string {
  const d = new Date(ts);
  return Number.isNaN(d.getTime()) ? ts : d.toLocaleTimeString();
}

function levelColor(level: string): string {
  switch (level.toLowerCase()) {
    case "error":
      return "var(--danger, #f87171)";
    case "warn":
      return "var(--warning, #f5a524)";
    default:
      return "var(--text-muted)";
  }
}

function StatusBadge({ status }: { status: string }) {
  const color =
    status === "ok"
      ? "var(--success, #22c55e)"
      : status === "failed" || status === "timeout"
        ? "var(--danger, #f87171)"
        : status === "cancelled" || status === "interrupted"
          ? "var(--text-muted)"
          : "var(--accent)";
  return (
    <span
      style={{
        fontSize: 12,
        fontWeight: 600,
        color,
        textTransform: "uppercase",
        letterSpacing: 0.4,
      }}
    >
      {status}
    </span>
  );
}
