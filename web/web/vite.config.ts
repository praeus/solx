import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      // @solx/http and @solx/surface live in the sibling solx-js repo and
      // are consumed straight from TS source — both are zero-dependency
      // and browser-safe, so there's no build step to keep in sync and no
      // npm package to install (bun's `file:`-dependency copy step hits a
      // reproducible EPERM on Windows for this repo layout; aliasing
      // sidesteps it entirely and stays live-editable besides).
      "@solx/http": fileURLToPath(new URL("../../solx-js/packages/http/src/index.ts", import.meta.url)),
      "@solx/surface": fileURLToPath(new URL("../../solx-js/packages/surface/src/index.ts", import.meta.url)),
      // @solx/widgets lives in the sibling solx-packages repo, same reason
      // as the two aliases above -- aliased straight to source rather than
      // installed, since it's consumed live and has no build step of its
      // own to keep in sync.
      "@solx/widgets/host": fileURLToPath(
        new URL("../../solx-packages/solx-widgets/src/host/index.ts", import.meta.url),
      ),
    },
  },
  server: {
    port: 5173,
    strictPort: false,
  },
  build: {
    outDir: "dist",
    sourcemap: true,
  },
});
