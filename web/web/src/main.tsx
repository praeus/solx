import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { App } from "./App";
import { IN_TAURI } from "./api/connection";
import { ConfigApp } from "./config/ConfigApp";
import "./index.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 2,
      retryDelay: (attempt) => Math.min(1000 * 2 ** attempt, 8000),
      refetchOnWindowFocus: false,
      staleTime: 30_000,
    },
  },
});

// The desktop app's Settings window loads this page as `?view=settings`.
// It renders only the config editor, never the main UI (which can mount
// package widgets): that window alone holds the `solx:config` permission.
const settingsView = IN_TAURI && new URLSearchParams(window.location.search).get("view") === "settings";

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("Root element #root not found");

createRoot(rootEl).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      {settingsView ? <ConfigApp /> : <App />}
    </QueryClientProvider>
  </StrictMode>,
);
