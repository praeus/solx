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
 */

import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useHealth } from "../api/hooks";
import { getStoredServerToken, getStoredServerUrl, setConnection } from "../api/connection";

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

  useEffect(() => {
    if (!open) return;
    setServerUrl(getStoredServerUrl());
    setServerToken(getStoredServerToken());
  }, [open]);

  if (!open) return null;

  const handleSave = () => {
    setConnection(serverUrl, serverToken);
    // The connection changed — cached query results (and any errors from
    // the previous connection, including "not configured") are stale.
    void queryClient.invalidateQueries();
    setSavedFlash(true);
    window.setTimeout(() => setSavedFlash(false), 2000);
  };

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
                ? `Connected to ${getStoredServerUrl()}`
                : `Not connected to ${getStoredServerUrl()}`
              : "Checking connection…"}
          </span>
        </div>

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
            Stored in this browser client only.
          </div>

          <div className="settings-panel__actions">
            <button
              className="settings-panel__btn settings-panel__btn--primary"
              onClick={handleSave}
            >
              Save
            </button>
            <button className="settings-panel__btn" onClick={onClose}>
              Close
            </button>
            {savedFlash && <span className="settings-panel__flash">Saved ✓</span>}
          </div>
        </div>
      </aside>
    </>
  );
}
