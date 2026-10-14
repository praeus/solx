/**
 * InstallTab — the official solx packages, one card each, installed or
 * uninstalled with a click (SettingsPanel's "Install" tab).
 *
 * The host does the work (`solx_packages::catalog` via `@solx/tauri`'s
 * `packages`): it lists the latest release of praeus/solx-packages (or the
 * folder in `SOLX_PACKAGES_SOURCE`), picks the zip for this platform,
 * installs whatever a package requires first, and runs its install.solx.
 * Only a local desktop host can do that, so SettingsPanel shows this tab
 * only there.
 */

import { useState } from "react";
import type { CatalogEntry } from "@solx/tauri";
import {
  useInstallPackage,
  usePackageCatalog,
  useRefreshPackageCatalog,
  useUninstallPackage,
} from "../../api/hooks";

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

function formatSize(bytes: number | null): string | null {
  if (!bytes) return null;
  return bytes >= 1024 * 1024
    ? `${(bytes / (1024 * 1024)).toFixed(1)} MB`
    : `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

const BADGES: Record<CatalogEntry["state"], { label: string; tone: string }> = {
  installed: { label: "Installed", tone: "ok" },
  update_available: { label: "Update available", tone: "warn" },
  available: { label: "Available", tone: "" },
  not_released: { label: "Not released", tone: "muted" },
  unsupported_platform: { label: "Not for this platform", tone: "muted" },
};

export function InstallTab() {
  const catalog = usePackageCatalog(true);
  const refresh = useRefreshPackageCatalog();
  const install = useInstallPackage();
  const uninstall = useUninstallPackage();

  const [busy, setBusy] = useState<{ name: string; label: string } | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [error, setError] = useState<{ name: string; message: string } | null>(null);
  const [warnings, setWarnings] = useState<{ name: string; list: string[] } | null>(null);

  const run = async (name: string, label: string, op: () => Promise<{ warnings: string[] }>) => {
    setBusy({ name, label });
    setError(null);
    setWarnings(null);
    setConfirming(null);
    try {
      const result = await op();
      if (result.warnings.length) setWarnings({ name, list: result.warnings });
    } catch (e) {
      setError({ name, message: errorText(e) });
    } finally {
      setBusy(null);
    }
  };

  if (catalog.isLoading) return <div className="dialog__hint">Loading packages…</div>;
  if (catalog.error || !catalog.data) {
    return (
      <div className="install-tab">
        <div className="dialog__hint settings-panel__error">
          Couldn't load the package catalog: {errorText(catalog.error)}
        </div>
        <button className="settings-panel__btn" onClick={() => void catalog.refetch()}>
          Retry
        </button>
      </div>
    );
  }

  const view = catalog.data;
  const recommended = view.entries.filter((e) => e.recommended);
  const others = view.entries.filter((e) => !e.recommended);

  const card = (entry: CatalogEntry) => {
    const badge = BADGES[entry.state];
    const isBusy = busy?.name === entry.name;
    const version = entry.installed_version ?? entry.available_version;
    const size = formatSize(entry.size);
    const doInstall = () =>
      void run(entry.name, entry.state === "update_available" ? "Updating…" : "Installing…", () =>
        install.mutateAsync(entry.name),
      );
    return (
      <div className="package-card" key={entry.name}>
        <div className="package-card__head">
          <span className="package-card__title">{entry.title}</span>
          <span className={`package-card__badge ${badge.tone}`}>{badge.label}</span>
        </div>
        <div className="package-card__name">
          {entry.name}
          {version && ` · v${version}`}
          {entry.state === "update_available" && entry.available_version && ` → v${entry.available_version}`}
          {size && entry.state !== "installed" && ` · ${size}`}
        </div>
        <div className="package-card__desc">{entry.description}</div>
        {entry.requires.length > 0 && (
          <div className="package-card__requires">Requires {entry.requires.join(", ")}</div>
        )}

        {confirming === entry.name ? (
          <div className="package-card__actions">
            <span className="package-card__confirm">Uninstall {entry.title}?</span>
            <button
              className="settings-panel__btn package-card__btn package-card__btn--danger"
              onClick={() =>
                void run(entry.name, "Uninstalling…", () => uninstall.mutateAsync(entry.name))
              }
            >
              Uninstall
            </button>
            <button className="settings-panel__btn package-card__btn" onClick={() => setConfirming(null)}>
              Cancel
            </button>
          </div>
        ) : (
          <div className="package-card__actions">
            {isBusy ? (
              <span className="package-card__busy">{busy.label}</span>
            ) : (
              <>
                {(entry.state === "available" || entry.state === "update_available") && (
                  <button
                    className="settings-panel__btn settings-panel__btn--primary package-card__btn"
                    disabled={busy !== null}
                    onClick={doInstall}
                  >
                    {entry.state === "update_available" ? "Update" : "Install"}
                  </button>
                )}
                {(entry.state === "installed" || entry.state === "update_available") && (
                  <button
                    className="settings-panel__btn package-card__btn"
                    disabled={busy !== null}
                    onClick={() => setConfirming(entry.name)}
                  >
                    Uninstall
                  </button>
                )}
                {(entry.state === "not_released" || entry.state === "unsupported_platform") && (
                  <button className="settings-panel__btn package-card__btn" disabled>
                    Install
                  </button>
                )}
              </>
            )}
          </div>
        )}

        {error?.name === entry.name && (
          <div className="dialog__hint settings-panel__error">{error.message}</div>
        )}
        {warnings?.name === entry.name && (
          <ul className="package-card__warnings">
            {warnings.list.map((w) => (
              <li key={w}>{w}</li>
            ))}
          </ul>
        )}
      </div>
    );
  };

  return (
    <div className="install-tab">
      <div className="install-tab__source">
        <span>
          {view.source}
          {view.tag && ` · ${view.tag}`}
          {` · ${view.platform ?? "unsupported platform"}`}
        </span>
        <button
          className="install-tab__refresh"
          onClick={() => refresh.mutate()}
          disabled={refresh.isPending || busy !== null}
          title="Check for a new release"
          aria-label="Check for a new release"
        >
          {refresh.isPending ? "…" : "↻"}
        </button>
      </div>
      {view.message && <div className="install-tab__message">{view.message}</div>}
      {refresh.error && (
        <div className="dialog__hint settings-panel__error">{errorText(refresh.error)}</div>
      )}

      <h3 className="install-tab__heading">Recommended</h3>
      <div className="install-tab__hint">Many other packages build on these.</div>
      {recommended.map(card)}

      <h3 className="install-tab__heading">All packages</h3>
      {others.map(card)}
    </div>
  );
}
