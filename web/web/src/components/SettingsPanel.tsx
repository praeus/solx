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
 * One field takes a connect code (`1234-5678`), a connect link or a token
 * (see `api/connectInput.ts`); a code is redeemed for this client's own
 * token at the server's `/auth/exchange` before anything is stored.
 *
 * In the solx desktop app the host owns the connection instead: a local
 * (server-build) host just reports where its embedded `solx-server` is
 * listening, and a client-build host takes the URL and credential via
 * `connectHost` — redeemed, verified and persisted on the Rust side, never
 * in `localStorage`.
 *
 * Config is still not exposed over HTTP, but a local desktop host can edit
 * it: "Open solx settings…" opens the separate Settings window
 * (`config/ConfigApp.tsx`), the only window allowed to touch it.
 *
 * In a browser, the status line also says what the stored token may do
 * (`GET /auth/whoami`), since a connect code may have been made for part
 * of solx only.
 *
 * Below the connection controls are tabs. "Install" (local desktop host
 * only) installs the official packages — see `settings/InstallTab.tsx`.
 * "Connect" is still to be designed.
 */

import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useHealth, useWhoami } from "../api/hooks";
import { describeScopes } from "../api/scopes";
import { InstallTab } from "./settings/InstallTab";
import { TabBar, type TabDefinition } from "./TabBar";
import {
  connectBrowser,
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
  const whoamiQuery = useWhoami();
  const queryClient = useQueryClient();

  const [serverUrl, setServerUrl] = useState("");
  const [credential, setCredential] = useState("");
  const [savedFlash, setSavedFlash] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [tab, setTab] = useState("install");

  // Re-read on every health refresh too, so the panel tracks the host.
  const host = IN_TAURI ? getHostInfo() : null;

  useEffect(() => {
    if (!open) return;
    setSaveError(null);
    if (IN_TAURI) {
      setServerUrl(getHostInfo()?.serverUrl ?? "");
      setCredential("");
    } else {
      setServerUrl(getStoredServerUrl());
      // Never load the saved token back into the UI; a blank field keeps it.
      setCredential("");
    }
  }, [open]);

  // Browser only: a token is already stored (it is never shown).
  const hasStoredToken = !IN_TAURI && getStoredServerToken().trim().length > 0;

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
    if (!IN_TAURI && !credential.trim()) {
      // Only the URL changed: keep the saved token.
      setConnection(serverUrl, getStoredServerToken());
      afterChange();
      return;
    }
    setSaving(true);
    try {
      if (IN_TAURI) await connectHost(serverUrl, credential);
      else await connectBrowser(serverUrl, credential);
      setCredential("");
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

  // Only a local desktop host can install packages.
  const tabs: TabDefinition[] = [
    ...(localHost ? [{ id: "install", label: "Install", enabled: true }] : []),
    { id: "connect", label: "Connect", enabled: true },
  ];
  const activeTab = tabs.some((t) => t.id === tab) ? tab : tabs[0]!.id;

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
        {whoamiQuery.data && (
          <div className="dialog__hint settings-panel__access">
            This browser's access: {describeScopes(whoamiQuery.data.scopes)}
            {whoamiQuery.data.scopes !== null && whoamiQuery.data.scope_enforcement !== "enforce" &&
              " (not enforced yet on this server)"}
          </div>
        )}

        {localHost ? (
          <div className="settings-panel__body settings-panel__body--top">
            <div className="dialog__hint">
              solx runs inside this app, so there is no server URL or token to
              enter here. Server options, including the embedded HTTP server,
              are in solx settings.
            </div>
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
        <div className="settings-panel__body settings-panel__body--top">
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
            <span className="settings-panel__label">Connect code or token</span>
            <input
              className="settings-panel__input"
              // A code is typed while reading it off another screen, so it
              // stays visible; anything else is masked like a password.
              type={/^[0-9 -]*$/.test(credential) ? "text" : "password"}
              value={credential}
              onChange={(e) => setCredential(e.target.value)}
              placeholder={
                hasStoredToken
                  ? "•••••••• saved — enter a code or token to replace it"
                  : "1234-5678, a connect link, or a token"
              }
              autoComplete="off"
              spellCheck={false}
            />
          </label>

          <div className="dialog__hint">
            Get a code on the solx host with <code>solx token connect-code</code>, or from the desktop
            app's settings ("Connect another device"). Pasting a connect link fills in the server too.{" "}
            {IN_TAURI
              ? "The token is checked, then stored in this computer's credential manager (not shared with other solx tools)."
              : "The token is stored in this browser only."}
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

        <TabBar
          tabs={tabs}
          activeTab={activeTab}
          onTabChange={setTab}
          ariaLabel="Settings sections"
          className="settings-panel__tabs"
        />
        <div className="settings-panel__tab" role="tabpanel">
          {activeTab === "install" ? (
            <InstallTab />
          ) : (
            <div className="dialog__hint">Connect is coming soon.</div>
          )}
        </div>
      </aside>
    </>
  );
}
