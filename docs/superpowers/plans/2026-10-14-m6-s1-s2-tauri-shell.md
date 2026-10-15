# M6 S1 + S2 — Tauri shell: tray, window lifecycle, engine wiring — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Subagents run on **sonnet** (CLAUDE.md).

**Status of record:** `HANDOFF.md`. This plan carries no running status of its own. Progress is recorded there, and in `mindmap.md`, `README.md` and `apps/desktop/README.md` (AGENTS.md §11).

**Goal:** Land slices S1 and S2 of M6. S1 is a Tauri v2 tray app that opens and destroys one window and only exits on Quit. S2 adds a real `ShellServices` implementation, the `Session` in managed state, an event pump to the webview, and the read-only IPC commands, with a Rust↔TS contract test.

**Architecture:** A new crate, `apps/desktop/crates/vrecnu-shell-tauri`, is the only crate that names Tauri. It owns `Arc<Session>`, implements `ShellServices` (`TauriShell`), and forwards `Session::subscribe()` to the webview from a shell-owned thread. A new pnpm package, `packages/desktop-ui`, is the frontend that crate embeds. It holds a TS mirror of the `vrecnu-shell-api` serde shapes and a pure reducer. A JSON fixture written by Rust and read by vitest pins the two together.

**Tech Stack:** Rust 1.90+ (workspace floor; see HANDOFF.md), Tauri 2 (`tray-icon`, `custom-protocol`), tauri-plugin-{single-instance, clipboard-manager, opener, notification}, crossbeam-channel. TS 5.9.3, React 19.2.8, Vite 8.2.0, Tailwind 4.3.3, vitest 4.1.10, @tauri-apps/api 2.

**Spec:** `docs/superpowers/specs/2026-10-09-m6-tauri-shell-design.md`. D1 and D2 were approved by the maintainer on 2026-10-14, and S1 and S2 are in scope. S3 to S5 get their own plan once S1's tray, exit and WebView2 behaviour and the exe size are known.

## Context

M0–M5 built the whole engine. `Session` implements every `RecorderEngine` method, and the publish pump already puts the share link on the clipboard through `ShellServices`. The project still cannot be used without a terminal. M6 is the shell that fixes that. S1 is a spike that finds out whether the tray, `prevent_exit` and window destroy in Tauri v2 behave as ADR-006 assumes. It also measures exe size early. S2 wires the engine in, with no recorder UI yet.

## Global Constraints

- Only `vrecnu-shell-tauri` may name `tauri`, `tao`, `wry` or `webview2-com`. `check-shell-boundary.ps1` stays unchanged, and nothing joins `$shellFreeCrates`.
- `@tauri-apps/*` may be imported only in `packages/desktop-ui`. An ESLint rule enforces this. `packages/publish` is not touched.
- No `window.__TAURI__` global: `withGlobalTauri` stays at its default of `false`.
- There is no JS permission for clipboard, opener or notification. Those plugins are used from Rust only. The capability grants event listen and unlisten, and nothing else.
- The window is destroyed on close, never hidden. With no window open, there is no WebView2 renderer.
- `provides_ipc_result_channel` stays `false`. There is no IPC result channel.
- Tauri crates are pinned with `=x.y.z` in the shell crate's own manifest, never in `[workspace.dependencies]`. `@tauri-apps/api` is pinned exactly, to the same minor version as the `tauri` crate.
- TS dependency versions match the existing packages exactly: react/react-dom 19.2.8, vite 8.2.0, @vitejs/plugin-react 6.0.5, tailwindcss and @tailwindcss/vite 4.3.3, vitest 4.1.10, @types/react 19.2.18, @types/react-dom 19.2.4.
- pnpm only. Run cargo from **PowerShell**, not Git Bash, which shadows `link.exe`.
- Build order: `pnpm --filter @vrecnu/desktop-ui build` must run before any cargo command that compiles the workspace, because `tauri::generate_context!` hard-fails on a missing `frontendDist`.
- Keep `.ps1` files ASCII-only. Make no `$ErrorActionPreference='Stop'` around cargo.
- Use conventional commits on branch `feat/m6-tauri-shell`, never on `main`. Every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Nothing is pushed without asking.
- `HANDOFF.md` is updated in the same change as anything that moves status.

## Review Focus

These five inputs and conditions come from the spec but are not exercised by its slice tests. Each one is pinned by a test in the task that owns it:

1. **A relaunch overwrites the previous launch's recording.** `next_session` starts at 1 in every process, and the output is `data_dir/vrecnu-<id>.mp4`. With S2's persistent data dir, the first recording after a restart would overwrite the last launch's first recording. Expected: ids never repeat across launches, and they stay below 2^53 because they cross IPC as JSON numbers. → Task 5.
2. **Launching the exe while it is already running.** Expected: the existing window is focused, with no second tray icon, no second `Session`, and no two publish servers racing for ports 47821–47825. → Task 3, using single-instance and the hardware check.
3. **Choosing "Open vrecnu" while the window is already open.** Expected: the existing window is focused and no duplicate is created. → Task 3, using `open_main` and the hardware check.
4. **An engine event arrives while no window exists, or `emit` fails.** Expected: the pump keeps running, so the next window still gets events. → Task 8 test.
5. **An event lands between the window subscribing and `get_snapshot` returning.** Expected: it is applied on top of the snapshot, not lost. → Task 10 reducer test.

---

## File map

**Create**
- `scripts/icon-art.mjs`: shared icon drawing, plus the PNG and ICO encoders. Moved from the web script.
- `packages/desktop-ui/`: `package.json`, `tsconfig.json`, `vite.config.ts`, `index.html`, `scripts/make-icons.mjs`, and these under `src/`:
  - `main.tsx`, `index.css`, `App.tsx`
  - `engine.ts` (TS mirror), `state.ts` (reducer), `tauri.ts` (invoke/listen bridge), `useEngine.ts`
  - `__fixtures__/engine.json` (generated by Rust)
  - `__tests__/contract.test.ts`, `__tests__/state.test.ts`
- `apps/desktop/crates/vrecnu-shell-tauri/`: `Cargo.toml`, `build.rs`, `tauri.conf.json`, `capabilities/default.json`, and `icons/` (generated, committed). Under `src/`:
  - `main.rs`, `window.rs`, `tray.rs`
  - `services.rs`, `events.rs`, `commands.rs`, `contract_tests.rs`
- `docs/superpowers/plans/2026-10-14-m6-s1-s2-tauri-shell.md`: a copy of this plan.

**Modify**
- `packages/web/scripts/make-icons.mjs`: imports from `scripts/icon-art.mjs`.
- `eslint.config.js`: adds desktop-ui to the browser/React block and adds the `@tauri-apps/*` ban.
- `.github/workflows/ci.yml`: the desktop job builds the publish page and desktop-ui first.
- `.gitignore`: adds `apps/desktop/crates/vrecnu-shell-tauri/gen/`.
- `apps/desktop/crates/vrecnu-engine/src/session.rs`: the session-id seed.
- `apps/desktop/crates/vrecnu-shell-api/src/lib.rs`: `rename_all_fields = "camelCase"` on `EngineEvent`.
- Docs:
  - `HANDOFF.md`
  - the spec's status line
  - `AGENTS.md` §4 (layout and boundary), §8 (build order) and §10 (the `@tauri-apps` rule)
  - `CLAUDE.md` (the "planned, not created yet" wording)
  - `apps/desktop/README.md`, `README.md` and `mindmap.md`

---

### Task 0: Branch, record approval, save the plan

**Files:** the spec's status line, `HANDOFF.md`, and `docs/superpowers/plans/2026-10-14-m6-s1-s2-tauri-shell.md`

- [ ] **Step 1:** Run `git switch -c feat/m6-tauri-shell`. It branches from `docs/status-of-record`, which is `main` plus one docs-only commit, `a39619a`, so the HANDOFF edits don't conflict.
- [ ] **Step 2:** Copy this plan verbatim to `docs/superpowers/plans/2026-10-14-m6-s1-s2-tauri-shell.md`.
- [ ] **Step 3:** Change the spec's `**Status:**` line to: `approved by the maintainer 2026-10-14 (D1 system browser, D2 packages/desktop-ui). S1+S2 planned in docs/superpowers/plans/2026-10-14-m6-s1-s2-tauri-shell.md.`
- [ ] **Step 4:** In `HANDOFF.md`:
  - Set "Last updated" to the current date.
  - Replace Next action 2 with "M6 in progress: design approved 2026-10-14; S1+S2 plan at `docs/superpowers/plans/2026-10-14-m6-s1-s2-tauri-shell.md`".
  - Change the M6 row's Notes to begin with "**In progress** (S1)".
  - Update `mindmap.md`'s ledger M6 entry and `README.md`'s status table to say "in progress".
- [ ] **Step 5:** Commit with `docs: approve M6 design (D1, D2) and plan S1+S2`.

---

### Task 1: `packages/desktop-ui` skeleton + lint boundary

**Files:**
- Create: `packages/desktop-ui/{package.json,tsconfig.json,vite.config.ts,index.html,src/main.tsx,src/index.css,src/App.tsx}`
- Modify: `eslint.config.js`, `pnpm-lock.yaml`

**Interfaces:** Produces `packages/desktop-ui/dist/index.html`. Task 3's `tauri.conf.json` embeds it.

- [ ] **Step 1: `package.json`**

```json
{
  "name": "@vrecnu/desktop-ui",
  "private": true,
  "version": "0.0.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "typecheck": "tsc -p tsconfig.json",
    "lint": "eslint . --max-warnings=0",
    "test": "vitest run --pool=threads --passWithNoTests"
  },
  "dependencies": {
    "react": "19.2.8",
    "react-dom": "19.2.8"
  },
  "devDependencies": {
    "@tailwindcss/vite": "4.3.3",
    "@types/react": "19.2.18",
    "@types/react-dom": "19.2.4",
    "@vitejs/plugin-react": "6.0.5",
    "tailwindcss": "4.3.3",
    "vite": "8.2.0",
    "vitest": "4.1.10"
  }
}
```

`--passWithNoTests` is only there until Task 10 adds tests, and Task 10 removes it.

- [ ] **Step 2: `tsconfig.json`, `vite.config.ts`, `index.html`**

```json
{
  "extends": "../../tsconfig.base.json",
  "compilerOptions": {
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "jsx": "react-jsx",
    "types": ["vite/client"]
  },
  "include": ["src", "vite.config.ts"]
}
```

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// vrecnu-shell-tauri embeds dist/ at compile time (tauri.conf.json → build.frontendDist), so
// this must be built before cargo compiles the desktop workspace. WebView2 is Chromium, so
// ES2022 needs no down-levelling.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  build: { target: "es2022" },
});
```

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>vrecnu</title>
  </head>
  <body class="bg-slate-950">
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

- [ ] **Step 3: `src/index.css`, `src/main.tsx`, `src/App.tsx`**

```css
@import "tailwindcss";
/* packages/ui lives outside this package's root, so Tailwind's auto-detection does not see it. */
@source "../../ui/src";
```

```tsx
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./index.css";

const container = document.getElementById("root");
if (!container) throw new Error("#root is missing from index.html");
createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
```

```tsx
export function App() {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-2 p-6 text-slate-100">
      <h1 className="text-xl font-semibold">vrecnu</h1>
      <p className="text-sm text-slate-400">Recorder controls arrive in a later slice.</p>
    </main>
  );
}
```

- [ ] **Step 4: ESLint**
  - In the browser/React block, add `"packages/desktop-ui/**/*.{ts,tsx}"` to `files`.
  - Add `"@tauri-apps/*"` to the core/storage block's `patterns`. That block has its own `no-restricted-imports`, and in flat config a later block would replace it rather than merge with it.
  - Insert this block immediately after the browser/React block:

```js
  // ADR-006: Tauri is confined to one crate on the Rust side and one package on the TS side.
  // This turns "no __TAURI__ branch in packages/publish" into a lint failure, not a review
  // comment. core/storage carry the same ban in their own block above, because a second
  // no-restricted-imports entry for those files would replace theirs rather than merge.
  {
    files: ["packages/**/*.{ts,tsx}"],
    ignores: ["packages/desktop-ui/**", "packages/core/**", "packages/storage/**"],
    rules: {
      "no-restricted-imports": [
        "error",
        { patterns: [{ group: ["@tauri-apps/*"], message: "Only packages/desktop-ui may import Tauri (ADR-006)." }] },
      ],
    },
  },
```

- [ ] **Step 5:** Run `pnpm install`. It updates `pnpm-lock.yaml`.
- [ ] **Step 6: Prove the ban fires.**
  1. Add `import "@tauri-apps/api/core";` to the top of `packages/publish/src/main.tsx`.
  2. Run `pnpm --filter @vrecnu/publish lint`. Expected: FAIL, with "Only packages/desktop-ui may import Tauri".
  3. Revert the line and re-run. Expected: PASS.
  4. Do the same check with `packages/storage/src/index.ts`. Expected: FAIL, then PASS.
- [ ] **Step 7:** Run `pnpm typecheck; pnpm lint; pnpm test; pnpm --filter @vrecnu/desktop-ui build`. Expected: all pass, and `packages/desktop-ui/dist/index.html` exists.
- [ ] **Step 8:** Commit with `feat(desktop-ui): scaffold the Tauri shell's frontend package and confine @tauri-apps imports to it`.

---

### Task 2: Shared icon art → desktop icons

**Files:**
- Create: `scripts/icon-art.mjs`, `packages/desktop-ui/scripts/make-icons.mjs`
- Modify: `packages/web/scripts/make-icons.mjs`, `packages/desktop-ui/package.json`
- Output (committed): `apps/desktop/crates/vrecnu-shell-tauri/icons/{icon.ico,32x32.png,128x128.png,128x128@2x.png,icon.png}`

`tauri-build` refuses to build on Windows without `icons/icon.ico`, and the tray uses the same image.

- [ ] **Step 1: Baseline.** Run `node packages/web/scripts/make-icons.mjs <scratch>/before`. `<scratch>` is the session scratchpad.
- [ ] **Step 2: Move the art.** Move `CRC_TABLE`, `crc32`, `chunk`, `encodePng` and `draw` verbatim from the web script into `scripts/icon-art.mjs`. Export `encodePng` and `draw`. Add this ICO encoder:

```js
/** Wraps PNGs in an ICO container (PNG-compressed entries, valid since Windows Vista). */
export function encodeIco(images) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(images.length, 4);
  const directory = Buffer.alloc(16 * images.length);
  let offset = header.length + directory.length;
  images.forEach(({ size, png }, i) => {
    const at = i * 16;
    directory[at] = size >= 256 ? 0 : size; // 0 means 256
    directory[at + 1] = size >= 256 ? 0 : size;
    directory.writeUInt16LE(1, at + 4); // colour planes
    directory.writeUInt16LE(32, at + 6); // bits per pixel
    directory.writeUInt32LE(png.length, at + 8);
    directory.writeUInt32LE(offset, at + 12);
    offset += png.length;
  });
  return Buffer.concat([header, directory, ...images.map((image) => image.png)]);
}
```

  The web script keeps its CLI part (the `outDir` and `targets` loop) and adds `import { draw, encodePng } from "../../../scripts/icon-art.mjs";`.
- [ ] **Step 3: Prove the refactor changes nothing.**
  1. Run `node packages/web/scripts/make-icons.mjs <scratch>/after`.
  2. Compare `Get-FileHash <scratch>/before/*` with `Get-FileHash <scratch>/after/*`. Expected: identical hashes.
- [ ] **Step 4: Desktop script** (`packages/desktop-ui/scripts/make-icons.mjs`):

```js
// Generates vrecnu-shell-tauri's icons from the same art as the PWA, with no image dependency.
// tauri-build embeds icons/icon.ico as the exe's resource, and the tray uses the same image.
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { draw, encodePng, encodeIco } from "../../../scripts/icon-art.mjs";

const outDir = process.argv[2];
mkdirSync(outDir, { recursive: true });
const png = (size) => encodePng(size, size, draw(size, { padding: 0, transparentBackground: true }));

for (const [name, size] of [["32x32.png", 32], ["128x128.png", 128], ["128x128@2x.png", 256], ["icon.png", 512]]) {
  writeFileSync(join(outDir, name), png(size));
}
const ico = encodeIco([16, 24, 32, 48, 64, 256].map((size) => ({ size, png: png(size) })));
writeFileSync(join(outDir, "icon.ico"), ico);
console.log(`icons written to ${outDir}`);
```

  Add the script `"icons": "node scripts/make-icons.mjs ../../apps/desktop/crates/vrecnu-shell-tauri/icons"` to desktop-ui's `package.json`.
- [ ] **Step 5:** Run `pnpm --filter @vrecnu/desktop-ui icons`. Expected: 5 files appear. Then run `pnpm lint`. Expected: PASS.
- [ ] **Step 6:** Commit with `chore(icons): share the icon art and generate the desktop shell's icons`.

---

### Task 3: `vrecnu-shell-tauri` S1: tray, window lifecycle, exit prevention, single instance

**Files:** these, under `apps/desktop/crates/vrecnu-shell-tauri/`:
- Create: `Cargo.toml`, `build.rs`, `tauri.conf.json`, `src/main.rs`, `src/window.rs`, `src/tray.rs`
- Modify: `.gitignore`

**Interfaces:**
- Produces `window::open_main(app: &tauri::AppHandle)` and `window::MAIN: &str = "main"`.
- Produces `tray::build(app: &tauri::AppHandle) -> tauri::Result<()>`.
- Tasks 8 and 9 add to `main.rs`'s `setup` and `invoke_handler`.

- [ ] **Step 1: Resolve exact versions.**
  1. Run `cargo search tauri --limit 1`, then the same for `tauri-build` and `tauri-plugin-single-instance`.
  2. Pin the newest 2.x of each with `=`.
  3. Record the versions in the commit message.
- [ ] **Step 2: `Cargo.toml`**

```toml
[package]
name = "vrecnu-shell-tauri"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "The Tauri v2 tray shell. The ONLY crate allowed to name tauri/tao/wry/webview2-com (ADR-006)."

[[bin]]
name = "vrecnu-shell-tauri"
path = "src/main.rs"

[features]
default = ["custom-protocol"]
# Serve the embedded packages/desktop-ui/dist rather than a dev-server URL. On by default so a
# plain `cargo run` is the real app: vrecnu builds with Cargo, not the Tauri CLI (AGENTS.md §8).
custom-protocol = ["tauri/custom-protocol"]

# Tauri is declared here and nowhere else. The workspace table deliberately does not list it
# (apps/desktop/Cargo.toml), so no other crate can pick it up with `.workspace = true`.
[build-dependencies]
tauri-build = { version = "=2.<resolved>", features = [] }

[dependencies]
tauri = { version = "=2.<resolved>", features = ["tray-icon"] }
tauri-plugin-single-instance = "=2.<resolved>"
tracing.workspace = true
tracing-subscriber.workspace = true
```

  Replace each `<resolved>` with the version from Step 1. A `<resolved>` left in place fails to parse, so it cannot slip through.
- [ ] **Step 3: `build.rs`**: `fn main() { tauri_build::build() }`.
- [ ] **Step 4: `tauri.conf.json`**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "vrecnu",
  "identifier": "io.github.jeswin-christie.vrecnu",
  "build": { "frontendDist": "../../../../packages/desktop-ui/dist" },
  "app": {
    "windows": [],
    "security": { "csp": "default-src 'self' ipc: http://ipc.localhost" }
  },
  "bundle": {
    "active": false,
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.ico"]
  }
}
```

  `"windows": []` means no window opens at startup (D3). Bundling stays off until M8. The identifier fixes the `%LOCALAPPDATA%` folder name, so it must not change after release.
- [ ] **Step 5: `src/window.rs`**

```rust
//! The one window. Created on demand and destroyed on close, never hidden: a hidden WebView2
//! keeps its renderer processes alive, which is the RSS ADR-006 is worried about (M9).

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// Label of the main window. Commands and capabilities refer to it by this name.
pub(crate) const MAIN: &str = "main";

/// Focuses the main window, creating it if it does not exist.
pub(crate) fn open_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        // Already open: focus it rather than stacking a duplicate.
        for result in [window.unminimize(), window.show(), window.set_focus()] {
            if let Err(err) = result {
                tracing::warn!(%err, "could not bring the window forward");
            }
        }
        return;
    }
    let built = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("vrecnu")
        .inner_size(400.0, 560.0)
        .resizable(false)
        .center()
        .build();
    if let Err(err) = built {
        tracing::error!(%err, "could not open the window");
    }
}
```

- [ ] **Step 6: `src/tray.rs`**

```rust
//! The tray icon: the app's only permanent surface (ADR-006's RSS mitigation).

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;

use crate::window;

const OPEN: &str = "open";
const QUIT: &str = "quit";

/// Installs the tray icon and its menu.
pub(crate) fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, OPEN, "Open vrecnu", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    let mut tray = TrayIconBuilder::with_id("vrecnu")
        .tooltip("vrecnu")
        .menu(&menu)
        // Left click opens the window; the menu is for right click.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            OPEN => window::open_main(app),
            // `exit` raises ExitRequested with a code, which main's run loop lets through.
            QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                window::open_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}
```

  If the pinned version names the builder method `menu_on_left_click` instead, use the name docs.rs gives for that version. Behaviour is the same.
- [ ] **Step 7: `src/main.rs`**

```rust
//! vrecnu's desktop shell (M6).
//!
//! **This is the only crate allowed to name `tauri`, `tao`, `wry` or `webview2-com`**
//! (ADR-006). Tauri is an output device and an input device here, not the application: the
//! engine runs the same with `vrecnu-cli` driving it, and `check-shell-boundary.ps1` keeps it so.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod tray;
mod window;

use tauri::RunEvent;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("VRECNU_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let app = tauri::Builder::default()
        // First, so a second launch is turned away before it builds anything of its own: two
        // instances would mean two tray icons and two publish servers racing for the same ports.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::open_main(app);
        }))
        .setup(|app| {
            tray::build(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(err) => {
            tracing::error!(%err, "vrecnu could not start");
            std::process::exit(1);
        }
    };

    app.run(|_app, event| {
        // Closing the last window raises ExitRequested with no code; the tray keeps the app
        // alive. Quit calls `exit(0)`, which carries a code and is let through.
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            api.prevent_exit();
        }
    });
}
```

  If `#![forbid(unsafe_code)]` fails with E0453 inside a Tauri macro expansion, change it to `#![deny(unsafe_code)]` and add a one-line comment naming the macro.
- [ ] **Step 8:** Append `apps/desktop/crates/vrecnu-shell-tauri/gen/` to `.gitignore`. `tauri-build` writes generated schemas there.
- [ ] **Step 9: Build and checks.** From `apps/desktop`, in PowerShell:
  1. `pnpm --filter @vrecnu/desktop-ui build`. This also needs `pnpm --filter @vrecnu/publish build`.
  2. `cargo fmt --all -- --check`
  3. `cargo clippy --all-targets --all-features -- -D warnings`
  4. `cargo test --workspace`. Still 240, because S1 adds no unit tests.
  5. `./scripts/check-shell-boundary.ps1 -ManifestPath (Resolve-Path Cargo.toml)`
  6. `cargo tree -p vrecnu-shell-tauri -e normal --prefix none | Select-String '^tauri v'`. Expected: one match.

  All must pass.
- [ ] **Step 10: Exe size** (spec risk: measure at the end of S1).
  1. `cargo build --release -p vrecnu-shell-tauri`
  2. `"{0:N2} MB" -f ((Get-Item target\release\vrecnu-shell-tauri.exe).Length / 1MB)`
  3. Record the figure against the ≤15 MB budget.
- [ ] **Step 11: Hardware checks (with the maintainer).**
  1. Run `target\debug\vrecnu-shell-tauri.exe`. Expected: a tray icon and no window.
  2. Left-click the icon. Expected: the window opens. Left-click again. Expected: the same window is focused, with no duplicate.
  3. Right-click and choose Open. Expected: focus. Right-click and choose Quit. Expected: the process exits.
  4. Relaunch. Close the window with ✕. Expected: the process is still alive and the tray icon remains.
  5. Launch the exe a second time. Expected: no second tray icon; the window opens on the first instance.
  6. **WebView2 teardown, 5 cycles.**
     1. Save `<scratch>/webviews.ps1`:

        ```powershell
        param([int]$ShellPid)
        $all = @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'")
        $tree = @($all | Where-Object ParentProcessId -eq $ShellPid); $frontier = $tree
        while ($frontier.Count) { $frontier = @($all | Where-Object { $frontier.ProcessId -contains $_.ParentProcessId }); $tree += $frontier }
        $tree.Count
        ```

     2. For each cycle, open the window (second launch or tray), then count. Expected: more than 0.
     3. Close it with ✕, wait 10 s, then count. **Expected: 0.**
     4. If the count does not return to 0, **stop**. Record the observation in HANDOFF as an S1 finding against ADR-006, and raise it with the maintainer. Do not work around it.
- [ ] **Step 12:** Commit with `feat(shell-tauri): S1 tray app with on-demand window, exit prevention and single instance`. List the pinned versions and the release exe size in the body.

---

### Task 4: CI builds the frontends first + S1 docs

**Files:** Modify `.github/workflows/ci.yml`, `HANDOFF.md`, `AGENTS.md` (§4, §8, §10), `CLAUDE.md`, `apps/desktop/README.md`, `README.md`, `mindmap.md`.

- [ ] **Step 1:** In the `desktop` job, insert these steps after checkout and before "Install Rust". They override the job's `apps/desktop` working directory.

```yaml
      # The desktop crates embed two Vite builds at compile time: the publish page
      # (vrecnu-publish-server, ADR-002) and the shell's UI (vrecnu-shell-tauri, whose
      # tauri::generate_context! fails outright if packages/desktop-ui/dist is missing).
      - uses: pnpm/action-setup@v4

      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: pnpm

      - name: Build the embedded frontends
        working-directory: .
        run: |
          pnpm install --frozen-lockfile
          pnpm --filter @vrecnu/publish --filter @vrecnu/desktop-ui build
```

- [ ] **Step 2: Docs.**
  - `AGENTS.md` §4: change the `vrecnu-shell-tauri/` line to "the Tauri v2 tray shell (M6, in progress) — the ONLY crate allowed to name Tauri". Add a `desktop-ui/` line under `packages/`: "the desktop shell's UI; the ONLY package allowed to import `@tauri-apps/*`".
  - `AGENTS.md` §8: add a build-trap bullet: "**Build `packages/desktop-ui` before any workspace cargo command.** `tauri::generate_context!` fails to compile without `packages/desktop-ui/dist`; `pnpm build` covers it." Change `cargo run -p vrecnu-cli -- help`'s comment and add `cargo run -p vrecnu-shell-tauri`.
  - `AGENTS.md` §10: add "Never import `@tauri-apps/*` outside `packages/desktop-ui` (lint-enforced)".
  - `CLAUDE.md`: change "(milestone M6 — planned, not created yet)" to "(milestone M6 — in progress)".
  - `apps/desktop/README.md`: add the build order and a Status line.
  - `HANDOFF.md`:
    - Add an "M6 — in progress" section with S1 results, each dated: the boundary check, `cargo tree`, the exe size, the five WebView2 cycles, and the tray/exit/single-instance observations.
    - Refresh "Verified state, right now" with new test counts and dates.
    - Update Next actions.
  - `mindmap.md` and `README.md`: "M6 S1 done".
- [ ] **Step 3: Rehearse the CI order locally from a clean state.** Nothing checks the YAML itself.
  1. Delete `packages/desktop-ui/dist` and `packages/publish/dist`.
  2. Run the step's two pnpm commands from the repo root.
  3. Run `cargo clippy --all-targets --all-features -- -D warnings` and `./scripts/check-shell-boundary.ps1 -ManifestPath (Resolve-Path Cargo.toml)` from `apps/desktop`. Expected: both pass.
- [ ] **Step 4:** Commit with `ci: build the embedded frontends before the desktop job; docs: record M6 S1`.

---

### Task 5: Engine: session ids never repeat across launches

**Files:** Modify `apps/desktop/crates/vrecnu-engine/src/session.rs`. That is the `next_session` initialiser in `Session::new` (line ~161) and its test module (line ~859).

**Interfaces:** Produces `SessionId` values ≥ the Unix-ms time at `Session::new`, and below 2^53.

- [ ] **Step 1: Failing test** (in `mod tests`):

```rust
#[test]
fn a_relaunch_does_not_reuse_the_previous_launchs_file_names() {
    // Ids name the output file (`vrecnu-<id>.mp4`) in a data directory that outlives the
    // process. Starting at 1 every launch overwrote the previous launch's first recording.
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let session = session();
    let first = session.next_session.load(Ordering::Relaxed);
    assert!(first >= before, "ids restart below the wall clock: {first} < {before}");
    // Ids cross IPC as JSON numbers; JavaScript loses integers above 2^53.
    assert!(first < 1 << 53);
}
```

- [ ] **Step 2:** Run `cargo test -p vrecnu-engine a_relaunch`. Expected: FAIL (`1 < before`).
- [ ] **Step 3: Implement.**

```rust
            // Seeded from the wall clock, not 1: the id names the output file, and the data
            // directory outlives the process. Milliseconds stay far below 2^53, so the id
            // survives the trip through JSON into the desktop UI.
            next_session: AtomicU64::new(first_session_id()),
```

  Add the helper next to `impl Session`:

```rust
/// Unix time in milliseconds; 1 if the clock reads before 1970.
fn first_session_id() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |elapsed| elapsed.as_millis() as u64)
}
```

- [ ] **Step 4:** Run `cargo test -p vrecnu-engine`. Expected: all pass, 48 tests. If an existing test assumed the first id is 1, update it to read the id from `start()`'s return value instead.
- [ ] **Step 5:** Commit with `fix(engine): seed session ids from the clock so a relaunch cannot overwrite an earlier recording`.

---

### Task 6: `EngineEvent` fields are camelCase on the wire

Every other boundary type is camelCase. `EngineEvent`'s `rename_all` renames only the variant tags, so fields go out as `bytes_sent` and `web_view_link`. No consumer exists yet, so this is the cheapest moment to fix it before the TS mirror locks it in.

**Files:** Modify `apps/desktop/crates/vrecnu-shell-api/src/lib.rs` (the `EngineEvent` attributes and the tests).

- [ ] **Step 1: Failing test** (in the shell-api `mod tests`):

```rust
#[test]
fn event_fields_are_camel_case_like_every_other_boundary_type() {
    let json = serde_json::to_value(EngineEvent::PublishDone {
        web_view_link: "link".into(),
        file_id: "id".into(),
    })
    .unwrap();
    assert_eq!(
        json,
        serde_json::json!({"type": "publishDone", "webViewLink": "link", "fileId": "id"})
    );
}
```

- [ ] **Step 2:** Run `cargo test -p vrecnu-shell-api event_fields`. Expected: FAIL.
- [ ] **Step 3:** Change the attribute to `#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]`. This needs serde ≥ 1.0.185; check `Cargo.lock`, and if it is older, run `cargo update -p serde`.
- [ ] **Step 4:** Run `cargo test -p vrecnu-shell-api`. Expected: 6 tests pass, and the existing `{"type":"elapsed","ms":1500}` test is unaffected. Then run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5:** Commit with `fix(shell-api): camelCase EngineEvent fields to match every other boundary type`.

---

### Task 7: `TauriShell`: `ShellServices` for real

**Files:** Create `src/services.rs`. Modify `Cargo.toml`, `src/main.rs` (add `mod services;`).

**Interfaces:**
- Produces `services::TauriShell::new(app: tauri::AppHandle) -> TauriShell`, which implements `vrecnu_shell_api::ShellServices`.
- Produces `services::resolve_ffmpeg(explicit: Option<OsString>, search_dirs: &[PathBuf]) -> PathBuf`.
- Produces `services::ensure_recordings_dir(base: &Path) -> vrecnu_shell_api::Result<PathBuf>`.

- [ ] **Step 1: Dependencies.** Resolve with `cargo search` and pin with `=`. Add to `[dependencies]`:
  - `tauri-plugin-clipboard-manager`, `tauri-plugin-opener`, `tauri-plugin-notification`
  - `vrecnu-types.workspace = true`, `vrecnu-shell-api.workspace = true`

  Add `tempfile = "3"` to `[dev-dependencies]`.
- [ ] **Step 2: Failing tests** (`services.rs`, `#[cfg(test)] mod tests`):

```rust
use super::*;

fn dir_with_ffmpeg() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(FFMPEG_FILE), b"").unwrap();
    dir
}

#[test]
fn an_explicit_override_wins_over_a_bundled_copy() {
    let bundled = dir_with_ffmpeg();
    let found = resolve_ffmpeg(Some("C:/x/ffmpeg.exe".into()), &[bundled.path().into()]);
    assert_eq!(found, PathBuf::from("C:/x/ffmpeg.exe"));
}

#[test]
fn an_empty_override_is_ignored_rather_than_spawning_an_empty_path() {
    assert_eq!(resolve_ffmpeg(Some(OsString::new()), &[]), PathBuf::from("ffmpeg"));
}

#[test]
fn the_first_directory_holding_ffmpeg_wins() {
    let empty = tempfile::tempdir().unwrap();
    let (first, second) = (dir_with_ffmpeg(), dir_with_ffmpeg());
    let dirs = [empty.path().into(), first.path().into(), second.path().into()];
    assert_eq!(resolve_ffmpeg(None, &dirs), first.path().join(FFMPEG_FILE));
}

#[test]
fn with_nothing_bundled_the_bare_name_is_left_for_path_lookup() {
    // Same contract as CliShell: the spawn failure names the path, not a guess made here.
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(resolve_ffmpeg(None, &[empty.path().into()]), PathBuf::from("ffmpeg"));
}

#[test]
fn the_recordings_folder_is_created_inside_the_data_directory_and_reused() {
    let base = tempfile::tempdir().unwrap();
    let dir = ensure_recordings_dir(base.path()).unwrap();
    assert_eq!(dir, base.path().join(RECORDINGS_DIR));
    assert!(dir.is_dir());
    assert_eq!(ensure_recordings_dir(base.path()).unwrap(), dir);
}

#[test]
fn an_unusable_data_directory_is_recoverable_and_loses_nothing() {
    let base = tempfile::tempdir().unwrap();
    let file = base.path().join("not-a-dir");
    std::fs::write(&file, b"").unwrap();
    let err = ensure_recordings_dir(&file).unwrap_err();
    assert!(err.recoverable);
    assert!(!err.session_lost);
}
```

- [ ] **Step 3:** Run `cargo test -p vrecnu-shell-tauri`. Expected: compile FAIL, because the items are undefined.
- [ ] **Step 4: Implement** (top of `services.rs`):

```rust
//! `ShellServices` for the Tauri shell. Everything here is called from Rust; the webview is
//! granted no clipboard, opener or notification permission (spec D4).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use vrecnu_shell_api::{AppError, NotifyLevel, Result, ShellServices};
use vrecnu_types::ErrorCode;

/// Overrides FFmpeg discovery, as it does for `vrecnu-cli`.
const FFMPEG_ENV: &str = "VRECNU_FFMPEG";
const FFMPEG_FILE: &str = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
const RECORDINGS_DIR: &str = "recordings";

/// The shell services the engine calls back into.
pub(crate) struct TauriShell {
    app: AppHandle,
}

impl TauriShell {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ShellServices for TauriShell {
    fn set_clipboard_text(&self, text: &str) -> Result<()> {
        self.app
            .clipboard()
            .write_text(text.to_owned())
            .map_err(|err| failure("vrecnu could not copy to the clipboard.", err))
    }

    fn open_external_url(&self, url: &str) -> Result<()> {
        self.app
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|err| failure("vrecnu could not open your browser.", err))
    }

    fn reveal_in_file_manager(&self, path: &Path) -> Result<()> {
        self.app
            .opener()
            .reveal_item_in_dir(path)
            .map_err(|err| failure("vrecnu could not open File Explorer.", err))
    }

    fn notify(&self, level: NotifyLevel, title: &str, body: &str) {
        match level {
            NotifyLevel::Error => tracing::error!("{title}: {body}"),
            NotifyLevel::Warning => tracing::warn!("{title}: {body}"),
            NotifyLevel::Info => tracing::info!("{title}: {body}"),
        }
        if let Err(err) = self.app.notification().builder().title(title).body(body).show() {
            tracing::warn!(%err, "could not show a notification");
        }
    }

    /// `VRECNU_FFMPEG`, then the resource dir (M8's bundled sidecar), then next to the exe,
    /// then `PATH` — the same order as `CliShell`, so the two shells cannot disagree.
    fn ffmpeg_path(&self) -> Result<PathBuf> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        let dirs: Vec<PathBuf> = [self.app.path().resource_dir().ok(), exe_dir]
            .into_iter()
            .flatten()
            .collect();
        Ok(resolve_ffmpeg(std::env::var_os(FFMPEG_ENV), &dirs))
    }

    fn data_dir(&self) -> Result<PathBuf> {
        let base = self
            .app
            .path()
            .app_local_data_dir()
            .map_err(|err| failure("vrecnu could not find a place to save recordings.", err))?;
        ensure_recordings_dir(&base)
    }
}

/// Picks the FFmpeg to spawn. Pure, so the precedence is testable without an app.
pub(crate) fn resolve_ffmpeg(explicit: Option<OsString>, search_dirs: &[PathBuf]) -> PathBuf {
    if let Some(path) = explicit.filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    search_dirs
        .iter()
        .map(|dir| dir.join(FFMPEG_FILE))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

/// `<base>/recordings`, created on first use.
pub(crate) fn ensure_recordings_dir(base: &Path) -> Result<PathBuf> {
    let dir = base.join(RECORDINGS_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|err| failure("vrecnu could not create its recordings folder.", err))?;
    Ok(dir)
}

/// Shell failures cost the user nothing already recorded: recoverable, never `session_lost`.
fn failure(message: &str, err: impl std::fmt::Display) -> AppError {
    AppError::recoverable(ErrorCode::Internal, message).with_detail(err.to_string())
}
```

  Register the plugins in `main.rs`, after single-instance: `.plugin(tauri_plugin_clipboard_manager::init()).plugin(tauri_plugin_opener::init()).plugin(tauri_plugin_notification::init())`. Until Task 8 uses `TauriShell`, declare the module as `#[allow(dead_code)] // wired in by the next commit` plus `mod services;` in `main.rs`, so the whole module is covered and `-D warnings` stays green. Remove the attribute in Task 8.
- [ ] **Step 5:** Run `cargo test -p vrecnu-shell-tauri`. Expected: 6 pass. Then run `cargo clippy -p vrecnu-shell-tauri --all-targets -- -D warnings`. Expected: clean.
- [ ] **Step 6:** Commit with `feat(shell-tauri): TauriShell implements ShellServices (clipboard, opener, notifications, ffmpeg, data dir)`.

---

### Task 8: Event pump + `Session` in managed state

**Files:** Create `src/events.rs`. Modify `Cargo.toml` (`vrecnu-engine.workspace = true`, `crossbeam-channel.workspace = true`, `serde.workspace = true`) and `src/main.rs`.

**Interfaces:**
- Produces `events::ENGINE_EVENT: &str = "engine-event"` and the trait `events::EventSink`.
- Produces `events::spawn_pump(events: Receiver<EngineEvent>, sink: impl EventSink) -> std::io::Result<JoinHandle<()>>` and `events::WebviewSink::new(AppHandle)`.
- Managed state: `Arc<vrecnu_engine::Session>`.

- [ ] **Step 1: Failing tests** (`events.rs`):

```rust
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Records what reached the "webview"; fails the sends listed in `fail_on`.
    #[derive(Clone, Default)]
    struct FakeSink {
        seen: Arc<Mutex<Vec<EngineEvent>>>,
        fail_on: Vec<usize>,
        calls: Arc<Mutex<usize>>,
    }

    impl EventSink for FakeSink {
        fn send(&self, event: &EngineEvent) -> std::result::Result<(), String> {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            if self.fail_on.contains(&*calls) {
                return Err("no window".into());
            }
            self.seen.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    #[test]
    fn events_reach_the_sink_in_order_and_the_pump_ends_with_the_engine() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let sink = FakeSink::default();
        let pump = spawn_pump(rx, sink.clone()).unwrap();
        tx.send(EngineEvent::Elapsed { ms: 1 }).unwrap();
        tx.send(EngineEvent::Elapsed { ms: 2 }).unwrap();
        drop(tx);
        pump.join().unwrap();
        assert_eq!(
            *sink.seen.lock().unwrap(),
            vec![EngineEvent::Elapsed { ms: 1 }, EngineEvent::Elapsed { ms: 2 }]
        );
    }

    #[test]
    fn a_failed_send_does_not_stop_the_pump() {
        // No window open (or one closing) must not cost the next window its events.
        let (tx, rx) = crossbeam_channel::unbounded();
        let sink = FakeSink { fail_on: vec![1], ..FakeSink::default() };
        let pump = spawn_pump(rx, sink.clone()).unwrap();
        tx.send(EngineEvent::Elapsed { ms: 1 }).unwrap();
        tx.send(EngineEvent::Elapsed { ms: 2 }).unwrap();
        drop(tx);
        pump.join().unwrap();
        assert_eq!(*sink.seen.lock().unwrap(), vec![EngineEvent::Elapsed { ms: 2 }]);
    }
}
```

- [ ] **Step 2:** Run `cargo test -p vrecnu-shell-tauri events`. Expected: compile FAIL.
- [ ] **Step 3: Implement.**

```rust
//! Forwards engine events to the webview.
//!
//! The pump is shell-owned: engine threads never see an `AppHandle` (ADR-006's second leak
//! path). Event *contents* are never logged — `PublishOpened` carries the capability token.

use std::thread::JoinHandle;

use crossbeam_channel::Receiver;
use tauri::{AppHandle, Emitter};
use vrecnu_shell_api::EngineEvent;

/// The single Tauri event the UI listens to. Its payload is `EngineEvent` as serialized.
pub(crate) const ENGINE_EVENT: &str = "engine-event";

/// Where pumped events go. A trait so the pump is testable without a webview.
pub(crate) trait EventSink: Send + 'static {
    fn send(&self, event: &EngineEvent) -> std::result::Result<(), String>;
}

/// Emits to every webview of the app.
pub(crate) struct WebviewSink(AppHandle);

impl WebviewSink {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self(app)
    }
}

impl EventSink for WebviewSink {
    fn send(&self, event: &EngineEvent) -> std::result::Result<(), String> {
        self.0.emit(ENGINE_EVENT, event).map_err(|err| err.to_string())
    }
}

/// Drains `events` into `sink` until the engine drops its sender.
pub(crate) fn spawn_pump(
    events: Receiver<EngineEvent>,
    sink: impl EventSink,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("vrecnu-event-pump".into())
        .spawn(move || {
            for event in events {
                if let Err(err) = sink.send(&event) {
                    tracing::warn!(%err, "an engine event did not reach the window");
                }
            }
        })
}
```

  In `main.rs`, add `mod events;` and replace `setup`:

```rust
        .setup(|app| {
            let shell = Arc::new(services::TauriShell::new(app.handle().clone()));
            let session = Arc::new(Session::new(shell));
            // Subscribe before anything can emit; the UI recovers older state from
            // get_snapshot, not from replayed events.
            let engine_events = RecorderEngine::subscribe(session.as_ref());
            events::spawn_pump(engine_events, events::WebviewSink::new(app.handle().clone()))?;
            app.manage(session);
            tray::build(app.handle())?;
            Ok(())
        })
```

  The imports are `std::sync::Arc`, `tauri::Manager`, `vrecnu_engine::Session` and `vrecnu_shell_api::RecorderEngine`. Remove the `#[allow(dead_code)]` from Task 7.
- [ ] **Step 4:** Run `cargo test -p vrecnu-shell-tauri`. Expected: 8 pass. Then run `cargo clippy --all-targets --all-features -- -D warnings`. Expected: clean.
- [ ] **Step 5:** Commit with `feat(shell-tauri): own the Session and pump its events to the webview`.

---

### Task 9: IPC commands + Rust side of the contract fixture

**Files:** Create `src/commands.rs` and `src/contract_tests.rs`. Modify `src/main.rs`. Generate `packages/desktop-ui/src/__fixtures__/engine.json`.

**Interfaces:**
- Produces commands `get_snapshot`, `list_sources`, `list_cameras` and `list_audio_devices`. They return `AppState`, `CaptureSource[]`, `CameraInfo[]` and `AudioDevices`, with `AppError` on failure.
- Produces a fixture with the shape `{ events: EngineEvent[12], appState, sources: CaptureSource[], cameras: CameraInfo[], audioDevices }`.

- [ ] **Step 1: `commands.rs`**

```rust
//! Thin IPC wrappers over `Arc<Session>`.
//!
//! All `async`: Tauri runs them on its worker pool, so a slow device enumeration (Media
//! Foundation, WASAPI) never blocks the event loop that drives the tray and the window.
//! `AppError` is `Serialize`, so `hint` and `session_lost` reach the UI intact.

use std::sync::Arc;

use tauri::State;
use vrecnu_engine::Session;
use vrecnu_shell_api::{AppError, AppState, RecorderEngine};
use vrecnu_types::{AudioDevices, CameraInfo, CaptureSource};

type Engine<'a> = State<'a, Arc<Session>>;

/// The UI calls this first on every window creation.
#[tauri::command(async)]
pub(crate) fn get_snapshot(engine: Engine<'_>) -> Result<AppState, AppError> {
    Ok(engine.snapshot())
}

#[tauri::command(async)]
pub(crate) fn list_sources(engine: Engine<'_>) -> Result<Vec<CaptureSource>, AppError> {
    engine.list_capture_sources()
}

#[tauri::command(async)]
pub(crate) fn list_cameras(engine: Engine<'_>) -> Result<Vec<CameraInfo>, AppError> {
    engine.list_cameras()
}

#[tauri::command(async)]
pub(crate) fn list_audio_devices(engine: Engine<'_>) -> Result<AudioDevices, AppError> {
    engine.list_audio_devices()
}
```

  In `main.rs`, add `mod commands;`, `#[cfg(test)] mod contract_tests;`, and before `.build(...)`:
  `.invoke_handler(tauri::generate_handler![commands::get_snapshot, commands::list_sources, commands::list_cameras, commands::list_audio_devices])`.
- [ ] **Step 2: `contract_tests.rs`**

```rust
//! Pins the JSON the webview receives. Writes packages/desktop-ui/src/__fixtures__/engine.json
//! with VRECNU_UPDATE_FIXTURES=1; otherwise fails if serialization drifted from it. The vitest
//! side (contract.test.ts) fails if engine.ts drifted from the same file.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Serialize;
use vrecnu_shell_api::{AppError, AppState, EngineEvent};
use vrecnu_types::{
    AudioDevice, AudioDevices, CameraInfo, CaptureSource, CaptureSourceId, CaptureSourceKind,
    CaptureStats, ErrorCode, Recording, RenderId, SessionId, SessionState, Size,
};

const UPDATE_ENV: &str = "VRECNU_UPDATE_FIXTURES";

/// Field order is fixed by the struct, not by serde_json's map (whose ordering depends on
/// workspace feature unification).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    events: Vec<EngineEvent>,
    app_state: AppState,
    sources: Vec<CaptureSource>,
    cameras: Vec<CameraInfo>,
    audio_devices: AudioDevices,
}

const ID: SessionId = SessionId(1_760_000_000_000);

fn error() -> AppError {
    AppError::recoverable(ErrorCode::PublishPortsBusy, "All five publish ports are in use.")
        .with_hint("Close whatever is using ports 47821-47825 and try again.")
        .with_detail("bind failed")
}

fn recording() -> Recording {
    Recording {
        session: ID,
        path: PathBuf::from("C:/recordings/vrecnu-1760000000000.mp4"),
        duration_ms: 10_000,
        size_bytes: 4_096,
        mime_type: "video/mp4".into(),
    }
}

fn stats() -> CaptureStats {
    // Exactly representable floats, so the fixture is stable across platforms.
    CaptureStats { frames_in: 300, frames_written: 300, frames_dropped: 0, frames_duplicated: 12, encoder_fps: 29.5 }
}

fn events() -> Vec<EngineEvent> {
    vec![
        EngineEvent::StateChanged { from: SessionState::Ready, to: SessionState::Recording },
        EngineEvent::Elapsed { ms: 1_500 },
        EngineEvent::AudioLevels { mic_db: -12.5, system_db: -30.25 },
        EngineEvent::Stats(stats()),
        EngineEvent::Warning { code: ErrorCode::CaptureBackpressure, message: "Frames are being dropped.".into() },
        EngineEvent::RenderProgress { id: RenderId(7), fraction: 0.25 },
        EngineEvent::RenderDone { id: RenderId(7), recording: recording() },
        EngineEvent::PublishOpened { url: "http://127.0.0.1:47821/?t=TOKEN".into(), port: 47_821 },
        EngineEvent::PublishProgress { bytes_sent: 1_024, total: 4_096 },
        EngineEvent::PublishDone { web_view_link: "https://drive.google.com/file/d/FILE_ID/view".into(), file_id: "FILE_ID".into() },
        EngineEvent::PublishFailed { error: error() },
        EngineEvent::Failed { error: error() },
    ]
}

/// No wildcard arm: a new variant fails to compile here until the fixture covers it.
fn variant(event: &EngineEvent) -> &'static str {
    match event {
        EngineEvent::StateChanged { .. } => "stateChanged",
        EngineEvent::Elapsed { .. } => "elapsed",
        EngineEvent::AudioLevels { .. } => "audioLevels",
        EngineEvent::Stats(_) => "stats",
        EngineEvent::Warning { .. } => "warning",
        EngineEvent::RenderProgress { .. } => "renderProgress",
        EngineEvent::RenderDone { .. } => "renderDone",
        EngineEvent::PublishOpened { .. } => "publishOpened",
        EngineEvent::PublishProgress { .. } => "publishProgress",
        EngineEvent::PublishDone { .. } => "publishDone",
        EngineEvent::PublishFailed { .. } => "publishFailed",
        EngineEvent::Failed { .. } => "failed",
    }
}

fn fixture() -> Fixture {
    Fixture {
        events: events(),
        app_state: AppState {
            session: SessionState::Stopped,
            session_id: Some(ID),
            elapsed_ms: 10_000,
            stats: stats(),
            recording: Some(recording()),
            error: None,
            publishing: false,
        },
        sources: vec![CaptureSource {
            id: CaptureSourceId("monitor:1".into()),
            title: "Display 1".into(),
            kind: CaptureSourceKind::Monitor,
            size: Size { width: 1920, height: 1080 },
            is_primary: true,
        }],
        cameras: vec![CameraInfo { id: "0".into(), name: "USB Camera".into() }],
        audio_devices: AudioDevices {
            inputs: vec![AudioDevice { id: "in".into(), name: "Microphone".into(), is_default: true }],
            outputs: vec![AudioDevice { id: "out".into(), name: "Speakers".into(), is_default: true }],
        },
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../packages/desktop-ui/src/__fixtures__/engine.json")
}

#[test]
fn the_fixture_has_one_event_of_every_variant() {
    let names: BTreeSet<_> = events().iter().map(variant).collect();
    assert_eq!(names.len(), events().len());
}

#[test]
fn the_desktop_ui_fixture_matches_what_the_engine_serializes() {
    let rendered = serde_json::to_string_pretty(&fixture()).unwrap() + "\n";
    let path = fixture_path();
    if std::env::var_os(UPDATE_ENV).is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &rendered).unwrap();
        return;
    }
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{}: {err}; regenerate with {UPDATE_ENV}=1", path.display()))
        .replace("\r\n", "\n");
    assert_eq!(
        on_disk, rendered,
        "the engine's JSON changed: regenerate with {UPDATE_ENV}=1 and update packages/desktop-ui/src/engine.ts"
    );
}
```

  Add `serde_json.workspace = true` to `[dev-dependencies]`.
- [ ] **Step 3:** Run `cargo test -p vrecnu-shell-tauri contract`. Expected: FAIL (the fixture is missing, and the panic names the env var).
- [ ] **Step 4:** Run `$env:VRECNU_UPDATE_FIXTURES='1'; cargo test -p vrecnu-shell-tauri contract; Remove-Item Env:VRECNU_UPDATE_FIXTURES`, then `cargo test -p vrecnu-shell-tauri`. Expected: 10 pass. Open `engine.json` and confirm:
  - fields are camelCase everywhere (Task 6)
  - `hint` and `detail` are present as strings
  - `stats` fields sit flat beside `"type"`
  - `id` is a plain string
  - `sessionId` is a plain number
- [ ] **Step 5:** Commit with `feat(shell-tauri): read-only IPC commands and the engine JSON contract fixture`, including `engine.json`.

---

### Task 10: desktop-ui: TS mirror, reducer, contract tests, diagnostic screen

**Files:**
- Create: under `packages/desktop-ui/src/`: `engine.ts`, `state.ts`, `tauri.ts`, `useEngine.ts`, `__tests__/contract.test.ts`, `__tests__/state.test.ts`
- Create: `apps/desktop/crates/vrecnu-shell-tauri/capabilities/default.json`
- Modify: `App.tsx`, `package.json` (add `@tauri-apps/api`, pinned exactly to the `tauri` crate's minor; drop `--passWithNoTests`)

**Interfaces:**
- Consumes the Task 9 fixture and command names, and `ENGINE_EVENT = "engine-event"`.
- Produces `reduce(store, action)`, `initialStore`, `Store`, `EngineView` and `PublishView`. S3 builds the recorder screens on these.

- [ ] **Step 1: `engine.ts`** (mirror):

```ts
// TS mirror of vrecnu-shell-api / vrecnu-types serde shapes. Rust writes
// __fixtures__/engine.json; __tests__/contract.test.ts fails if this file disagrees with it.

export type SessionState = "idle" | "ready" | "recording" | "finalizing" | "stopped" | "failed";

export type ErrorCode =
  | "CAPTURE_UNAVAILABLE" | "CAPTURE_PERMISSION" | "CAPTURE_SOURCE_GONE" | "CAPTURE_BACKPRESSURE"
  | "AUDIO_NO_LOOPBACK" | "AUDIO_DEVICE_LOST" | "CAMERA_BUSY" | "CAMERA_NONE" | "CAMERA_PERMISSION"
  | "ENCODE_MISSING_BINARY" | "ENCODE_FAILED" | "ENCODE_DIED" | "PUBLISH_PORTS_BUSY"
  | "PUBLISH_ASSETS_MISSING" | "PUBLISH_TIMEOUT" | "PUBLISH_PAGE_ERROR" | "DISK_FULL"
  | "ILLEGAL_STATE" | "INTERNAL";

export interface AppError {
  code: ErrorCode;
  message: string;
  hint: string | null;
  detail: string | null;
  recoverable: boolean;
  sessionLost: boolean;
}

export interface CaptureStats {
  framesIn: number;
  framesWritten: number;
  framesDropped: number;
  framesDuplicated: number;
  encoderFps: number;
}

export interface Recording {
  session: number;
  path: string;
  durationMs: number;
  sizeBytes: number;
  mimeType: string;
}

export interface AppState {
  session: SessionState;
  sessionId: number | null;
  elapsedMs: number;
  stats: CaptureStats;
  recording: Recording | null;
  error: AppError | null;
  publishing: boolean;
}

export interface CaptureSource {
  id: string;
  title: string;
  kind: "monitor" | "window";
  size: { width: number; height: number };
  isPrimary: boolean;
}

export interface CameraInfo {
  id: string;
  name: string;
}

export interface AudioDevice {
  id: string;
  name: string;
  isDefault: boolean;
}

export interface AudioDevices {
  inputs: AudioDevice[];
  outputs: AudioDevice[];
}

export type EngineEvent =
  | { type: "stateChanged"; from: SessionState; to: SessionState }
  | { type: "elapsed"; ms: number }
  | { type: "audioLevels"; micDb: number; systemDb: number }
  | ({ type: "stats" } & CaptureStats)
  | { type: "warning"; code: ErrorCode; message: string }
  | { type: "renderProgress"; id: number; fraction: number }
  | { type: "renderDone"; id: number; recording: Recording }
  | { type: "publishOpened"; url: string; port: number }
  | { type: "publishProgress"; bytesSent: number; total: number }
  | { type: "publishDone"; webViewLink: string; fileId: string }
  | { type: "publishFailed"; error: AppError }
  | { type: "failed"; error: AppError };

export type EngineEventType = EngineEvent["type"];
```

- [ ] **Step 2: `state.ts`** (pure reducer):

```ts
import type { AppError, AppState, EngineEvent, ErrorCode } from "./engine";

export type PublishView =
  | { phase: "idle" }
  | { phase: "open" }
  | { phase: "uploading"; bytesSent: number; total: number }
  | { phase: "done"; webViewLink: string }
  | { phase: "failed"; error: AppError };

export interface EngineView {
  app: AppState;
  levels: { micDb: number; systemDb: number } | null;
  render: { id: number; fraction: number } | null;
  publish: PublishView;
  warnings: { code: ErrorCode; message: string }[];
}

/** Events that arrive before the first snapshot are buffered, then applied on top of it. */
export type Store =
  | { status: "loading"; buffered: EngineEvent[] }
  | { status: "ready"; view: EngineView };

export type Action = { kind: "snapshot"; state: AppState } | { kind: "event"; event: EngineEvent };

export const initialStore: Store = { status: "loading", buffered: [] };
export const MAX_WARNINGS = 5;

export function reduce(store: Store, action: Action): Store {
  if (action.kind === "snapshot") {
    const seeded = fromSnapshot(action.state);
    // A snapshot is authoritative: a late one (window re-created) replaces the view outright.
    const view = store.status === "loading" ? store.buffered.reduce(apply, seeded) : seeded;
    return { status: "ready", view };
  }
  if (store.status === "loading") {
    return { status: "loading", buffered: [...store.buffered, action.event] };
  }
  return { status: "ready", view: apply(store.view, action.event) };
}

function fromSnapshot(app: AppState): EngineView {
  return {
    app,
    levels: null,
    render: null,
    publish: app.publishing ? { phase: "open" } : { phase: "idle" },
    warnings: [],
  };
}

function apply(view: EngineView, event: EngineEvent): EngineView {
  switch (event.type) {
    case "stateChanged":
      return { ...view, app: { ...view.app, session: event.to } };
    case "elapsed":
      return { ...view, app: { ...view.app, elapsedMs: event.ms } };
    case "audioLevels":
      return { ...view, levels: { micDb: event.micDb, systemDb: event.systemDb } };
    case "stats":
      return {
        ...view,
        app: {
          ...view.app,
          stats: {
            framesIn: event.framesIn,
            framesWritten: event.framesWritten,
            framesDropped: event.framesDropped,
            framesDuplicated: event.framesDuplicated,
            encoderFps: event.encoderFps,
          },
        },
      };
    case "warning":
      return {
        ...view,
        warnings: [...view.warnings, { code: event.code, message: event.message }].slice(-MAX_WARNINGS),
      };
    case "renderProgress":
      return { ...view, render: { id: event.id, fraction: event.fraction } };
    case "renderDone":
      return { ...view, render: null, app: { ...view.app, recording: event.recording } };
    case "publishOpened":
      return { ...view, app: { ...view.app, publishing: true }, publish: { phase: "open" } };
    case "publishProgress":
      return { ...view, publish: { phase: "uploading", bytesSent: event.bytesSent, total: event.total } };
    case "publishDone":
      return { ...view, app: { ...view.app, publishing: false }, publish: { phase: "done", webViewLink: event.webViewLink } };
    case "publishFailed":
      return { ...view, app: { ...view.app, publishing: false }, publish: { phase: "failed", error: event.error } };
    case "failed":
      return { ...view, app: { ...view.app, error: event.error } };
  }
}
```

- [ ] **Step 3: Failing tests.**
  - `__tests__/contract.test.ts` imports `fixture from "../__fixtures__/engine.json"` and casts it to `{ events: EngineEvent[]; appState: AppState; sources: CaptureSource[]; cameras: CameraInfo[]; audioDevices: AudioDevices }` via `unknown`.
  - It declares, with the types tied by `satisfies`:
    - `EVENT_KEYS: { [K in EngineEventType]: readonly (keyof Extract<EngineEvent, { type: K }>)[] }`, listing every field per variant (`stats` lists `type` plus the five `CaptureStats` fields)
    - `APP_STATE_KEYS`, `APP_ERROR_KEYS`, `RECORDING_KEYS`, `STATS_KEYS`, `SOURCE_KEYS`, `CAMERA_KEYS` and `AUDIO_DEVICE_KEYS`
  - It adds a compile-time completeness check:

    ```ts
    type Missing = { [K in EngineEventType]: Exclude<keyof Extract<EngineEvent, { type: K }>, (typeof EVENT_KEYS)[K][number]> }[EngineEventType];
    const noEventFieldUnlisted: [Missing] extends [never] ? true : false = true;
    ```

    It repeats the same pattern for `AppState`.
  - Its tests are:
    - (a) `noEventFieldUnlisted` is `true`
    - (b) the sorted fixture `type`s equal the sorted keys of `EVENT_KEYS`
    - (c) `it.each` over the events: the sorted `Object.keys(event)` equal the sorted `EVENT_KEYS[event.type]`
    - (d) `appState`, `appState.stats` and `appState.recording` key sets match. So do the `error` of the `failed` event, `sources[0]`, `cameras[0]` and `audioDevices.inputs[0]`.
    - (e) the reducer applies each fixture event on top of `reduce(initialStore, { kind: "snapshot", state: appState })`. The expected results per type are:
      - `stateChanged`: `session` is `"recording"`
      - `elapsed`: `elapsedMs` is 1500
      - `audioLevels`: `levels` is `{ micDb: -12.5, systemDb: -30.25 }`
      - `stats`: `framesDuplicated` is 12 and `encoderFps` is 29.5
      - `warning`: `warnings[0].code` is `"CAPTURE_BACKPRESSURE"`
      - `renderProgress`: `render` is `{ id: 7, fraction: 0.25 }`
      - `renderDone`: `recording.mimeType` is `"video/mp4"`
      - `publishOpened`: `publishing` is `true`
      - `publishProgress`: `publish` is `{ phase: "uploading", bytesSent: 1024, total: 4096 }`
      - `publishDone`: `publish.webViewLink` is `"https://drive.google.com/file/d/FILE_ID/view"`
      - `publishFailed`: `publish.phase` is `"failed"` and `error.sessionLost` is `false`
      - `failed`: `app.error.code` is `"PUBLISH_PORTS_BUSY"`
  - `__tests__/state.test.ts` covers:
    - (1) an `elapsed` event dispatched before the snapshot is buffered, and is applied after the snapshot arrives (Review Focus 5)
    - (2) a second snapshot while ready replaces the view and clears warnings
    - (3) after 7 warnings only the newest 5 remain
    - (4) opened → progress → done leaves `publishing === false` and `publish.phase === "done"`
    - (5) a snapshot with `publishing: true` seeds `publish.phase === "open"`
  - Remove `--passWithNoTests` from the test script.
- [ ] **Step 4:** Run `pnpm --filter @vrecnu/desktop-ui test`. Expected: FAIL before Steps 1–2 exist. If they were written first, temporarily rename a field in `engine.ts` (`elapsed.ms` → `elapsed.millis`) to watch typecheck and the tests fail, then revert.
- [ ] **Step 5: Bridge, hook, diagnostic screen, capability.**

```ts
// tauri.ts — the only module that touches @tauri-apps/api.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppState, AudioDevices, CameraInfo, CaptureSource, EngineEvent } from "./engine";

/** Must match vrecnu-shell-tauri's events::ENGINE_EVENT. */
export const ENGINE_EVENT = "engine-event";

export const getSnapshot = () => invoke<AppState>("get_snapshot");
export const listSources = () => invoke<CaptureSource[]>("list_sources");
export const listCameras = () => invoke<CameraInfo[]>("list_cameras");
export const listAudioDevices = () => invoke<AudioDevices>("list_audio_devices");

export function onEngineEvent(handler: (event: EngineEvent) => void): Promise<UnlistenFn> {
  return listen<EngineEvent>(ENGINE_EVENT, (message) => handler(message.payload));
}
```

```ts
// useEngine.ts
import { useEffect, useReducer } from "react";
import { initialStore, reduce, type Store } from "./state";
import { getSnapshot, onEngineEvent } from "./tauri";

/** Listens first, then reads the snapshot: an event landing in between is buffered, not lost. */
export function useEngine(): Store {
  const [store, dispatch] = useReducer(reduce, initialStore);
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    onEngineEvent((event) => dispatch({ kind: "event", event }))
      .then(async (stop) => {
        if (cancelled) return stop();
        unlisten = stop;
        dispatch({ kind: "snapshot", state: await getSnapshot() });
      })
      .catch((err: unknown) => console.error("vrecnu: could not connect to the engine", err));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
  return store;
}
```

  `App.tsx` becomes a diagnostic screen. It renders `store.view.app` as JSON (or "connecting…"). It also renders a device summary from `listSources`, `listCameras` and `listAudioDevices`, via `Promise.all` in an effect with `.catch` showing the `AppError` JSON. Sources show `id`, `kind` and `size` only, never window titles. Keep the Task 1 layout classes. Add `@tauri-apps/api` and run `pnpm install`.

  `capabilities/default.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Least privilege: listen to engine-event. App commands are allowed by default; no plugin permissions are granted to JS.",
  "windows": ["main"],
  "permissions": ["core:event:allow-listen", "core:event:allow-unlisten"]
}
```

- [ ] **Step 6:** Run `pnpm typecheck; pnpm lint; pnpm test; pnpm build`. Expected: all pass, and desktop-ui's test count is about 25.
- [ ] **Step 7:** Commit with `feat(desktop-ui): engine TS mirror, snapshot-seeded reducer and the contract tests that pin both to Rust`.

---

### Task 11: S2 verification on hardware + docs

- [ ] **Step 1: Full checks.**
  1. From the repo root: `pnpm typecheck; pnpm lint; pnpm test; pnpm build`
  2. From `apps/desktop` in PowerShell: `cargo fmt --all -- --check`
  3. `cargo clippy --all-targets --all-features -- -D warnings`
  4. `cargo test --workspace`. Expected: 240 + 1 (engine) + 1 (shell-api) + 10 (shell-tauri) = **252**.
  5. `./scripts/check-shell-boundary.ps1 -ManifestPath (Resolve-Path Cargo.toml)`
  6. `cargo run -p vrecnu-cli -- help`, plus `cargo test -p vrecnu-cli`, which must still report 20.
- [ ] **Step 2: Hardware, with the maintainer.**
  1. Run `cargo run -p vrecnu-shell-tauri` and open the window from the tray.
  2. Expected: the snapshot JSON shows `"session": "idle"`, and the device summary lists at least one monitor plus the real camera and audio counts.
  3. Open DevTools (right-click → Inspect, debug build). Expected: no CSP violations and no IPC permission errors.
  4. Close and reopen. Expected: the snapshot renders again.
  5. Re-run the five-cycle WebView2 teardown check from Task 3.
- [ ] **Step 3: Docs.**
  - `HANDOFF.md`:
    - Add the S2 results to the M6 section, each dated. Include the new test counts, the camelCase event change, the session-id seed fix, and the hardware observations.
    - Refresh "Verified state, right now" (TS counts too).
    - Set Next actions to "S3–S5 plan".
  - `mindmap.md`, `README.md`, `apps/desktop/README.md`: "M6 S1+S2 done".
- [ ] **Step 4:** Commit with `docs: record M6 S2 verification`.
- [ ] **Step 5:** Run the `superpowers:requesting-code-review` whole-branch review against AGENTS.md (boundary, scope, no secrets). Then ask the maintainer about pushing and opening a PR. Do not push unasked.

---

## Verification summary (exit proof for this plan)

1. `check-shell-boundary.ps1` passes unchanged, and `cargo tree -p vrecnu-shell-tauri` is the only graph containing `tauri`.
2. `vrecnu-cli` builds and its 20 tests pass. The workspace runs 252 Rust tests.
3. `pnpm typecheck`, `lint`, `test` and `build` pass. The ESLint ban is proven to fire in `packages/publish` and `packages/storage`.
4. The contract fixture is round-tripped: Rust regenerates it identically, and vitest validates keys and reducer behaviour against it.
5. On hardware:
   - The tray appears, with no window at startup.
   - Closing the window keeps the app alive, and Quit exits.
   - A second launch focuses the first instance.
   - **0 WebView2 processes remain 10 s after closing, across 5 cycles.**
   - The S2 diagnostic window shows a live snapshot and devices.
6. The release exe size is recorded against the ≤15 MB budget.
7. HANDOFF, mindmap, README and apps/desktop/README reflect S1 and S2 with dated evidence.
