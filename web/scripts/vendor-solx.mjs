#!/usr/bin/env bun
/**
 * scripts/vendor-solx.mjs
 *
 * Copies the built solx-js packages from ../solx-js/packages/ into
 * server/vendor/ and rewrites their package.json files so the `workspace:*`
 * dependencies become `file:` paths to siblings.  This lets `solx-web` use
 * the solx SDK without needing the full solx-js monorepo in its install
 * closure.
 *
 * Run: `bun run scripts/vendor-solx.mjs`
 */

import { cp, mkdir, readdir, readFile, rm, writeFile, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, "..");
const SOLX_JS_ROOT = resolve(ROOT, "..", "solx-js");
const VENDOR_DIR = resolve(ROOT, "server", "vendor");
const PLATFORM_PKG_DIR = join(SOLX_JS_ROOT, "platform-packages", "solx-win32-x64-msvc");

const PACKAGES = [
  "surface",
  "types",
  "files",
  "docs",
  "config",
  "actions",
  "scripts",
  "sdk",
];

const PLATFORM_PACKAGES = [
  "solx-win32-x64-msvc",
];

async function main() {
  console.log("Vendoring solx-js packages into", VENDOR_DIR);

  // Clean previous vendor
  await rm(VENDOR_DIR, { recursive: true, force: true });
  await mkdir(VENDOR_DIR, { recursive: true });

  // Copy each package, rewriting its package.json to use file: paths
  for (const pkg of PACKAGES) {
    const src = join(SOLX_JS_ROOT, "packages", pkg);
    if (!existsSync(src)) {
      console.warn(`  [skip] ${pkg}: not found at ${src}`);
      continue;
    }
    const dest = join(VENDOR_DIR, pkg);
    await cp(src, dest, { recursive: true, filter: (src) => {
      // Skip node_modules, dist build artifacts
      const base = src.split(/[\\\/]/).pop();
      if (base === "node_modules") return false;
      if (base === "test" && src.endsWith("/test")) return false;
      return true;
    }});

    // Rewrite package.json: workspace:* → file:./<sibling>
    const pkgPath = join(dest, "package.json");
    if (existsSync(pkgPath)) {
      const raw = await readFile(pkgPath, "utf8");
      const json = JSON.parse(raw);
      rewriteDeps(json);
      await writeFile(pkgPath, JSON.stringify(json, null, 2) + "\n");
    }
    console.log(`  [ok] ${pkg}`);
  }

  // Copy platform packages (containing solx.node binary)
  for (const platform of PLATFORM_PACKAGES) {
    const src = join(SOLX_JS_ROOT, "platform-packages", platform);
    if (!existsSync(src)) {
      console.warn(`  [skip] ${platform}: not found at ${src}`);
      continue;
    }
    const dest = join(VENDOR_DIR, platform);
    const binaryDest = join(dest, "solx.node");

    // If the existing vendored binary is bigger than the placeholder
    // (i.e. it's a real Rust build, not the 19MB placeholder), keep it
    // and only refresh the package.json.  This lets `cargo build` of the
    // bindings crate land in vendor/ without being clobbered on the next
    // `bun run dev`.
    let preserveBinary = false;
    if (existsSync(binaryDest)) {
      const [existingStat, placeholderStat] = await Promise.all([
        stat(binaryDest),
        stat(join(src, "solx.node")).catch(() => null),
      ]);
      if (placeholderStat && existingStat.size > placeholderStat.size * 1.2) {
        preserveBinary = true;
      }
    }

    await cp(src, dest, {
      recursive: true,
      filter: (file) => {
        if (preserveBinary && file.endsWith("solx.node")) return false;
        return true;
      },
    });
    console.log(
      `  [ok] ${platform} (platform binary${preserveBinary ? ", kept existing build" : ""})`,
    );
  }

  console.log("\nDone. Vendored solx-js packages at:", VENDOR_DIR);
}

function rewriteDeps(json) {
  for (const field of ["dependencies", "optionalDependencies", "peerDependencies"]) {
    const deps = json[field];
    if (!deps) continue;
    for (const [name, version] of Object.entries(deps)) {
      if (typeof version === "string" && version.startsWith("workspace:")) {
        if (name === "@solx/win32-x64-msvc" || name.startsWith("@solx/")) {
          // Local file: path within vendor
          const subpkg = name.replace(/^@solx\//, "");
          deps[name] = `file:./${subpkg}`;
        }
      }
    }
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
