// Publish the built web UI (web/web/dist) to a solx-server's file store, under
// `web/`, where the server serves it without a token at `<server>/ui/`.
//
//   bun scripts/publish-ui.mjs [--build] [--maps]
//
// Writing `web/` needs the root token (anything else is refused, since code
// there runs next to every browser user's token). The token is
// `SOLX_SERVER_TOKEN`, else `solx token show`'s. The server is
// `SOLX_SERVER_URL`, else http://127.0.0.1:8766: the root token is only
// accepted from the server's own machine, so this normally runs there.
//
// Order matters to a browser loading the UI mid-publish: assets go first,
// `index.html` (which names them) next, and only then are files the new
// build no longer has deleted.
import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const WEB_DIR = fileURLToPath(new URL("../web", import.meta.url));
const DIST = join(WEB_DIR, "dist");
const PREFIX = "web"; // solx_files::WEB_UI_DIR

const args = new Set(process.argv.slice(2));
const server = (process.env.SOLX_SERVER_URL ?? "http://127.0.0.1:8766").replace(/\/+$/, "");

function fail(message) {
  console.error(`publish-ui: ${message}`);
  process.exit(1);
}

function rootToken() {
  if (process.env.SOLX_SERVER_TOKEN?.trim()) return process.env.SOLX_SERVER_TOKEN.trim();
  const out = spawnSync("solx", ["token", "show"], { encoding: "utf8", shell: true });
  if (out.status !== 0) {
    fail(`couldn't run \`solx token show\` (${out.stderr?.trim() || "not on PATH?"}); set SOLX_SERVER_TOKEN`);
  }
  try {
    return JSON.parse(out.stdout).token;
  } catch {
    fail("`solx token show` didn't print the expected JSON; set SOLX_SERVER_TOKEN");
  }
}

function walk(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)],
  );
}

/** `/files/web/<rel>`, each segment encoded. */
function fileUrl(rel) {
  return `${server}/files/${rel.split("/").map(encodeURIComponent).join("/")}`;
}

async function call(method, url, token, body) {
  const res = await fetch(url, { method, headers: { Authorization: `Bearer ${token}` }, body });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    fail(`${method} ${url}: HTTP ${res.status} ${text}`);
  }
  return res;
}

if (args.has("--build")) {
  const built = spawnSync("bun", ["run", "build"], { cwd: WEB_DIR, stdio: "inherit", shell: true });
  if (built.status !== 0) fail("build failed");
}

let local;
try {
  local = walk(DIST)
    .map((p) => relative(DIST, p).split("\\").join("/"))
    .filter((rel) => args.has("--maps") || !rel.endsWith(".map"));
} catch {
  fail(`${DIST} doesn't exist; build first (or pass --build)`);
}
if (!local.includes("index.html")) fail(`${DIST} has no index.html`);

const token = rootToken();
const ordered = [...local.filter((rel) => rel !== "index.html"), "index.html"];
for (const rel of ordered) {
  await call("PUT", fileUrl(`${PREFIX}/${rel}`), token, readFileSync(join(DIST, rel)));
}

const listed = await call("GET", `${server}/files?prefix=${PREFIX}`, token);
const { paths } = await listed.json();
const keep = new Set(local.map((rel) => `${PREFIX}/${rel}`));
const stale = paths.filter((p) => !keep.has(p));
for (const rel of stale) await call("DELETE", fileUrl(rel), token);

console.log(
  `Published ${local.length} files to ${server}/ui/` +
    (stale.length ? ` (removed ${stale.length} from the previous build)` : ""),
);
