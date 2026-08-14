/**
 * TabBar — entity type tabs across the top.
 *
 * For now, only Documents is active. The other tabs are rendered as
 * disabled placeholders with a "Coming soon" tooltip.
 */

import type { CSSProperties } from "react";

export interface TabDefinition {
  id: string;
  label: string;
  enabled: boolean;
}

export const DEFAULT_TABS: TabDefinition[] = [
  { id: "documents", label: "Documents", enabled: true },
  { id: "actions", label: "Actions", enabled: false },
  { id: "types", label: "Types", enabled: false },
  { id: "files", label: "Files", enabled: false },
];

interface Props {
  tabs?: TabDefinition[];
  activeTab: string;
  onTabChange: (tabId: string) => void;
}

export function TabBar({ tabs = DEFAULT_TABS, activeTab, onTabChange }: Props) {
  return (
    <div className="tab-bar" role="tablist" aria-label="Entity types">
      {tabs.map((tab) => {
        const isActive = tab.id === activeTab;
        const style: CSSProperties = {
          opacity: tab.enabled ? 1 : 0.4,
          cursor: tab.enabled ? "pointer" : "not-allowed",
          borderBottomColor: isActive ? "var(--accent)" : "transparent",
          color: isActive ? "var(--text)" : "var(--text-muted)",
        };
        return (
          <button
            key={tab.id}
            role="tab"
            aria-selected={isActive}
            aria-disabled={!tab.enabled}
            disabled={!tab.enabled}
            title={tab.enabled ? undefined : "Coming soon"}
            className="tab-bar__tab"
            style={style}
            onClick={() => {
              if (tab.enabled) onTabChange(tab.id);
            }}
          >
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}
