// Start solx-web's Vite dev server (the app's devUrl), unless one is already
// serving on its port. Used by both `tauri dev` (beforeDevCommand) and the
// VS Code "ui dev" task, so running one after the other doesn't fail with
// "Port 5173 is already in use".
//
// When a server is already up, this prints a ready line (so the VS Code
// background task's problemMatcher sees it start and finish) and then idles
// until it's stopped, leaving that server alone. Otherwise it runs Vite and,
// when stopped, kills Vite's whole process tree — on Windows, killing just
// `bun` leaves `vite`/`node` orphaned, still holding the port.
import { spawn, spawnSync } from "node:child_process";
import { connect } from "node:net";
import { fileURLToPath } from "node:url";

const PORT = 5173; // solx-web's vite.config.ts (strictPort) and tauri.conf.json's devUrl
const URL_ = `http://localhost:${PORT}/`;
const WEB_DIR = fileURLToPath(new URL("../../solx-web/web", import.meta.url));

/** Whether anything is listening on the port. Vite binds `localhost`, which
 * may be only `::1` (or only 127.0.0.1), so try both. A TCP connect rather
 * than an HTTP request: a cold Vite can take seconds to answer one. */
function listening(host) {
  return new Promise((resolve) => {
    const socket = connect({ host, port: PORT });
    const done = (result) => {
      socket.destroy();
      resolve(result);
    };
    socket.setTimeout(500, () => done(false));
    socket.once("connect", () => done(true));
    socket.once("error", () => done(false));
  });
}

async function serving() {
  const results = await Promise.all(["::1", "127.0.0.1"].map(listening));
  return results.some(Boolean);
}

if (await serving()) {
  console.log(`VITE dev server already running at ${URL_} — reusing it (ready in 0 ms)`);
  setInterval(() => {}, 1 << 30);
} else {
  const child = spawn("bun", ["run", "dev"], { cwd: WEB_DIR, stdio: "inherit" });

  const stop = () => {
    if (child.exitCode !== null) process.exit(0);
    if (process.platform === "win32") {
      spawnSync("taskkill", ["/pid", String(child.pid), "/T", "/F"], { stdio: "ignore" });
    } else {
      child.kill("SIGTERM");
    }
  };
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP", "SIGBREAK"]) process.on(signal, stop);
  process.on("exit", stop);

  child.on("exit", (code) => process.exit(code ?? 0));
}
