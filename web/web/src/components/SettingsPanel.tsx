/**
 * SettingsPanel — slide-out panel for configuring the solx-server connection.
 *
 * The backend reads SOLX_SERVER_URL / SOLX_SERVER_TOKEN from its own
 * environment at startup, so the settings saved here persist via the config
 * service and are loaded by the backend on its next restart. (In a follow-up
 * the backend could read from config.snapshot() at runtime instead of env.)
 *
 * For the v1 UI, this panel shows what the backend currently has plus a
 * snapshot of the local config dir. Patching server URL/token is wired
 * through patchConfig; on success we show a toast and the form is read-only
 * until the backend picks up the new value.
 */

import { useEffect, useState } from "react";
import { useConfig, useMutateConfig, useHealth } from "../api/hooks";

interface Props {
  open: boolean;
  onClose: () => void;
}

export function SettingsPanel({ open, onClose }: Props) {
  const configQuery = useConfig();
  const mutate = useMutateConfig();
  const healthQuery = useHealth();

  const [serverUrl, setServerUrl] = useState("");
  const [serverToken, setServerToken] = useState("");
  const [dataDirectory, setDataDirectory] = useState("");
  const [filesDirectory, setFilesDirectory] = useState("");
  const [savedFlash, setSavedFlash] = useState(false);

  useEffect(() => {
    if (!configQuery.data) return;
    setServerUrl(String(configQuery.data.serverUrl ?? ""));
    setServerToken(String(configQuery.data.serverToken ?? ""));
    setDataDirectory(String(configQuery.data.dataDirectory ?? ""));
    setFilesDirectory(String(configQuery.data.filesDirectory ?? ""));
  }, [configQuery.data]);

  if (!open) return null;

  const handleSave = async () => {
    try {
      await mutate.mutateAsync({
        serverUrl: serverUrl.trim() || undefined,
        serverToken: serverToken.trim() || undefined,
        dataDirectory: dataDirectory.trim() || undefined,
        filesDirectory: filesDirectory.trim() || undefined,
      });
      setSavedFlash(true);
      window.setTimeout(() => setSavedFlash(false), 2000);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      window.alert(`Failed to save settings: ${message}`);
    }
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
            className={`settings-panel__status-dot${healthQuery.data?.solxConnected ? " ok" : " err"}`}
            aria-hidden
          />
          <span>
            {healthQuery.data
              ? healthQuery.data.solxConnected
                ? `Connected to ${healthQuery.data.serverUrl}`
                : `Not connected to ${healthQuery.data.serverUrl ?? "solx-server"}`
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
              placeholder="bearer token (if required)"
              autoComplete="off"
            />
          </label>

          <label className="settings-panel__field">
            <span className="settings-panel__label">Data Directory</span>
            <input
              className="settings-panel__input"
              type="text"
              value={dataDirectory}
              onChange={(e) => setDataDirectory(e.target.value)}
              placeholder="e.g. C:\Users\you\AppData\Roaming\praeus\solx"
              spellCheck={false}
            />
          </label>

          <label className="settings-panel__field">
            <span className="settings-panel__label">Files Directory</span>
            <input
              className="settings-panel__input"
              type="text"
              value={filesDirectory}
              onChange={(e) => setFilesDirectory(e.target.value)}
              placeholder="e.g. C:\Users\you\AppData\Roeless\praeus\solx\files"
              spellCheck={false}
            />
          </label>

          <div className="settings-panel__actions">
            <button
              className="settings-panel__btn settings-panel__btn--primary"
              onClick={handleSave}
              disabled={mutate.isPending}
            >
              {mutate.isPending ? "Saving…" : "Save"}
            </button>
            <button className="settings-panel__btn" onClick={onClose}>
              Close
            </button>
            {savedFlash && <span className="settings-panel__flash">Saved ✓</span>}
            {mutate.isError && (
              <span className="settings-panel__error">
                Error: {mutate.error instanceof Error ? mutate.error.message : String(mutate.error)}
              </span>
            )}
          </div>
        </div>
      </aside>
    </>
  );
}
