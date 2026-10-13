/**
 * SettingsPanel — slide-out panel for configuring the solx-server connection.
 *
 * There is no backend anymore to hold this for us — the browser itself
 * holds `serverUrl`/`serverToken` (in `localStorage`, via `connection.ts`)
 * and talks to `solx-server` directly through `@solx/http`. Everything
 * else the old panel exposed (`dataDirectory`/`filesDirectory`, full
 * `solx.config.*` editing) has no HTTP equivalent and never will — config
 * is local-only to a running `solx-server`/`solx-cli` process by design —
 * so those fields are gone, not moved.
 *
 * In the solx desktop app the host owns the connection instead: a local
 * (server-build) host just reports where its embedded `solx-server` is
 * listening, and a client-build host takes the URL/token via `connectHost`
 * — verified and persisted on the Rust side, never in `localStorage`.
 *
 * Config is still not exposed over HTTP, but a local desktop host can edit
 * it: "Open solx settings…" opens the separate Settings window
 * (`config/ConfigApp.tsx`), the only window allowed to touch it.
 */

import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useHealth } from "../api/hooks";
import {
  connectHost,
  connectionLabel,
  disconnectHost,
  getHostInfo,
  getStoredServerToken,
  getStoredServerUrl,
  IN_TAURI,
  openSettingsWindow,
  setConnection,
} from "../api/connection";

interface Props {
  open: boolean;
  onClose: () => void;
}

export function SettingsPanel({ open, onClose }: Props) {
  const healthQuery = useHealth();
  const queryClient = useQueryClient();

  const [serverUrl, setServerUrl] = useState("");
  const [serverToken, setServerToken] = useState("");
  const [savedFlash, setSavedFlash] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // Re-read on every health refresh too, so the panel tracks the host.
  const host = IN_TAURI ? getHostInfo() : null;

  useEffect(() => {
    if (!open) return;
    setSaveError(null);
    if (IN_TAURI) {
      setServerUrl(getHostInfo()?.serverUrl ?? "");
      setServerToken("");
    } else {
      setServerUrl(getStoredServerUrl());
      setServerToken(getStoredServerToken());
    }
  }, [open]);

  if (!open) return null;

  const afterChange = () => {
    // The connection changed — cached query results (and any errors from
    // the previous connection, including "not configured") are stale.
    void queryClient.invalidateQueries();
    setSavedFlash(true);
    window.setTimeout(() => setSavedFlash(false), 2000);
  };

  const handleSave = async () => {
    setSaveError(null);
    if (!IN_TAURI) {
      setConnection(serverUrl, serverToken);
      afterChange();
      return;
    }
    setSaving(true);
    try {
      await connectHost(serverUrl, serverToken);
      setServerToken("");
      afterChange();
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleDisconnect = async () => {
    setSaveError(null);
    try {
      await disconnectHost();
      afterChange();
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  };

  // A local desktop host has nothing to configure here: solx runs in the
  // app, and its embedded server (if enabled) is set up in solx-config.json.
  const localHost = host?.mode === "local";

  return (
    <>
      <div className="settings-backdrop" onClick={onClose} />
      <aside className="settings-panel" role="dialog" aria-label="Settings">
        <div className="settings-panel__header">
          <h2 className="settings-panel__title">Settings</h2>
          <button className="settings-panel__close" onClick={onClose} aria-label="Close settings">
            ✕
          </button>
        </div>

        <div className="settings-panel__status">
          <span
            className={`settings-panel__status-dot${healthQuery.data?.reachable ? " ok" : " err"}`}
            aria-hidden
          />
          <span>
            {healthQuery.data
              ? healthQuery.data.reachable
                ? `Connected to ${connectionLabel()}`
                : `Not connected to ${connectionLabel()}`
              : "Checking connection…"}
          </span>
        </div>

        {localHost ? (
          <div className="settings-panel__body">
            {host?.embeddedServer ? (
              <>
                <div className="dialog__hint">
                  This app also serves solx over HTTP for other clients
                  (browsers, solx-cli, the desktop client):
                </div>
                <label className="settings-panel__field">
                  <span className="settings-panel__label">Server URL</span>
                  <input className="settings-panel__input" type="text" readOnly value={host.embeddedServer.url} />
                </label>
                <label className="settings-panel__field">
                  <span className="settings-panel__label">Server Token</span>
                  <input
                    className="settings-panel__input"
                    type="text"
                    readOnly
                    value={host.embeddedServer.token}
                    onFocus={(e) => e.currentTarget.select()}
                  />
                </label>
              </>
            ) : (
              <div className="dialog__hint">
                solx runs inside this app. The embedded HTTP server is off; turn
                it on under HTTP server in solx settings.
              </div>
            )}
            {saveError && <div className="dialog__hint settings-panel__error">{saveError}</div>}
            <div className="settings-panel__actions">
              <button
                className="settings-panel__btn settings-panel__btn--primary"
                onClick={() =>
                  void openSettingsWindow().catch((e: unknown) =>
                    setSaveError(e instanceof Error ? e.message : String(e)),
                  )
                }
              >
                Open solx settings…
              </button>
              <button className="settings-panel__btn" onClick={onClose}>
                Close
              </button>
            </div>
          </div>
        ) : (
        <div className="settings-panel__body">
          <label className="settings-panel__field">
            <span className="settings-panel__label">Server URL</span>
            <input
              className="settings-panel__input"
              type="text"
              value={serverUrl}
              onChange={(e) => setServerUrl(e.target.value)}
              placeholder="http://127.0.0.1:8766"
              spellCheck={false}
            />
          </label>

          <label className="settings-panel__field">
            <span className="settings-panel__label">Server Token</span>
            <input
              className="settings-panel__input"
              type="password"
              value={serverToken}
              onChange={(e) => setServerToken(e.target.value)}
              placeholder="bearer token (printed by solx-server on first start)"
              autoComplete="off"
            />
          </label>

          <div className="dialog__hint">
            {IN_TAURI
              ? "Checked, then stored by this app (not shared with other solx tools)."
              : "Stored in this browser client only."}
          </div>
          {saveError && <div className="dialog__hint settings-panel__error">{saveError}</div>}

          <div className="settings-panel__actions">
            <button
              className="settings-panel__btn settings-panel__btn--primary"
              onClick={() => void handleSave()}
              disabled={saving}
            >
              {saving ? "Connecting…" : IN_TAURI ? "Connect" : "Save"}
            </button>
            {IN_TAURI && host?.mode === "remote" && (
              <button className="settings-panel__btn" onClick={() => void handleDisconnect()}>
                Disconnect
              </button>
            )}
            <button className="settings-panel__btn" onClick={onClose}>
              Close
            </button>
            {savedFlash && <span className="settings-panel__flash">Saved ✓</span>}
          </div>
        </div>
        )}
      </aside>
    </>
  );
}
