/**
 * ActionRunner — run an action detached, watch its console live, cancel it.
 *
 * Unlike ActionEditor's plain "Run Action" (a single synchronous `exec`
 * request/response round trip), this drives the async invocation primitives
 * (`solx-actions/src/invocations.rs`, dispatched as ordinary Internal
 * actions under `/builtin/action/*` and `/builtin/console/*` — see
 * `api.ts`'s doc comment on that section):
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
 *
 * Note: `@solx/http`'s manager methods (mirroring `solx-surface`'s
 * `ActionManager` trait 1:1) take no cancellation token, so unlike the old
 * Bun-backend version of this component, closing the dialog no longer
 * aborts the in-flight long-poll `fetch` itself — it just stops the loop
 * from acting on the response once it lands (`stoppedRef`). The request
 * against `solx-server` still runs to completion (up to `POLL_WAIT_SECS`/
 * `TAIL_WAIT_SECS`), same as it always did once detached; only the local
 * JS-side bookkeeping is skipped.
 */

import { useEffect, useRef, useState } from "react";
import {
  isTerminalInvocationStatus,
  pollInvocation,
  startInvocation,
  stopInvocation,
  tailConsole,
  type ConsoleEntry,
  type Invocation,
} from "../api/api";

interface Props {
  open: boolean;
  onClose: () => void;
  action: { path: string; name: string };
}

const POLL_WAIT_SECS = 20;
const TAIL_WAIT_SECS = 20;

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function ActionRunner({ open, onClose, action }: Props) {
  const [paramsText, setParamsText] = useState("{}");
  const [invocation, setInvocation] = useState<Invocation | null>(null);
  const [consoleEntries, setConsoleEntries] = useState<ConsoleEntry[]>([]);
  const [startError, setStartError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [stopping, setStopping] = useState(false);

  // `stoppedRef` tears down both watch loops (dialog closed / unmounted).
  // `runningRef` tracks just the invocation's own liveness so the console
  // loop knows when to stop long-polling and drain to a final zero-wait read.
  const stoppedRef = useRef(false);
  const runningRef = useRef(false);
  const consoleBoxRef = useRef<HTMLDivElement | null>(null);

  // Reset for a fresh run whenever the dialog opens (or the target action
  // changes while it's open).
  useEffect(() => {
    if (!open) return;
    setParamsText("{}");
    setInvocation(null);
    setConsoleEntries([]);
    setStartError(null);
    setStarting(false);
    setStopping(false);
  }, [open, action.path, action.name]);

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
      } catch (err) {
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
      } catch (err) {
        if (stoppedRef.current) return;
        await sleep(1500);
      }
    }
  };

  const handleStart = async () => {
    setStartError(null);
    let params: unknown = {};
    if (paramsText.trim()) {
      try {
        params = JSON.parse(paramsText);
      } catch (e) {
        setStartError(`Params JSON is invalid: ${(e as Error).message}`);
        return;
      }
    }
    setStarting(true);
    setInvocation(null);
    setConsoleEntries([]);
    stoppedRef.current = false;
    runningRef.current = true;
    try {
      const inv = await startInvocation(action.path, action.name, params);
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

  if (!open) return null;

  const status = invocation?.status ?? null;
  const terminal = status ? isTerminalInvocationStatus(status) : false;
  const canStart = starting || !invocation || terminal;
  const canStop = !!invocation && !terminal;

  return (
    <>
      <div className="dialog-backdrop" onClick={onClose} />
      <div className="dialog dialog--wide" role="dialog" aria-label="Run action">
        <div className="dialog__header">
          <h2 className="dialog__title">
            Run: {action.path}/{action.name}
          </h2>
          <button className="dialog__close" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>

        <div className="dialog__body">
          <div className="form-field">
            <label className="form-label">Params (JSON)</label>
            <textarea
              className="form-textarea form-json"
              rows={4}
              value={paramsText}
              onChange={(e) => setParamsText(e.target.value)}
              disabled={!canStart}
              spellCheck={false}
            />
          </div>

          <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
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
              {stopping ? "…" : status === "cancelling" ? "Force stop" : "Cancel"}
            </button>
            {status && <StatusBadge status={status} />}
          </div>

          {startError && <div className="dialog__error">{startError}</div>}

          {invocation && (
            <div className="form-field" style={{ marginTop: 8 }}>
              <label className="form-label">Console</label>
              <div
                ref={consoleBoxRef}
                style={{
                  height: 220,
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
                      <span>{entry.message ?? (entry.data != null ? JSON.stringify(entry.data) : "")}</span>
                    </div>
                  ))
                )}
              </div>
            </div>
          )}

          {invocation && terminal && (
            <div className="form-field" style={{ marginTop: 8 }}>
              <label className="form-label">{invocation.error ? "Error" : "Result"}</label>
              <textarea
                className="form-textarea form-json"
                rows={6}
                readOnly
                value={
                  invocation.error ? invocation.error : JSON.stringify(invocation.result ?? null, null, 2)
                }
                spellCheck={false}
              />
            </div>
          )}
        </div>

        <div className="dialog__footer">
          <button className="dialog__btn" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </>
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
