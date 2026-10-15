# vrecnu desktop (Phase 2)

The Rust side of vrecnu. See [ADR-006](../../docs/adr/006-phase-2-tauri-shell-with-swappable-core.md)
for the shell decision and [`read_this_idiot.md`](../../read_this_idiot.md) for the escape
hatch back to a pure-Rust GUI.

## Prerequisites

Rust stable (**1.90 or newer**, one floor for the whole workspace, required by Tauri 2.12)
**and the Windows SDK**.

> **The Windows SDK is not optional and its absence is confusing.** Without it, `cargo build`
> — and even `cargo check`, as soon as a build script or proc-macro needs recompiling — fails
> with:
>
> ```
> LINK : fatal error LNK1181: cannot open input file 'kernel32.lib'
> ```
>
> or `error: linker 'link.exe' not found`. Visual Studio can be installed *with the MSVC
> compiler but without the SDK*, which is exactly the state that produces this: `link.exe`
> exists, so the error looks like a project problem rather than a missing component.
>
> `cargo check` fails too, which surprises people: proc-macro crates (`serde_derive`,
> `thiserror-impl`) and build scripts are compiled for the host and must actually link.

Install the C++ desktop workload, which includes the SDK:

```powershell
# Elevated prompt. Adjust --installPath if VS is not Community 2022.
& "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vs_installer.exe" modify `
    --installPath "C:\Program Files\Microsoft Visual Studio\2022\Community" `
    --add Microsoft.VisualStudio.Workload.NativeDesktop --includeRecommended
```

Or in the Visual Studio Installer GUI: **Modify -> Desktop development with C++**.

Verify:

```powershell
Test-Path "${env:ProgramFiles(x86)}\Windows Kits\10\Lib"   # must be True
```

Avoid running `cargo` from Git Bash: its `/usr/bin/link` (GNU coreutils) shadows MSVC's
`link.exe` and produces a misleading `link: extra operand` error. PowerShell or cmd is fine —
rustc locates the SDK through the registry, so a Developer shell is not required.

### If the build dies with `memory allocation of N bytes failed`

Not a code problem. Cargo runs one `rustc` per logical CPU (16 on a typical laptop), and each
can want hundreds of megabytes. Straight after a Visual Studio or SDK install, `TiWorker` and
Defender are still indexing gigabytes of new files, and committed memory runs out:

```
memory allocation of 2097152 bytes failed
... (exit code: 0xc0000409, STATUS_STACK_BUFFER_OVERRUN)
```

Cap the parallelism and it builds fine:

```powershell
cargo test --workspace -j 2
```

Once the installer settles, full parallelism works again.

## Commands

**Build the frontends first** (from the repo root). The desktop crates embed two Vite builds at
compile time: `packages/publish/dist` (the publish page) and `packages/desktop-ui/dist` (the
shell's UI). `tauri::generate_context!` fails to compile without the latter, so this is required
before any workspace cargo command:

```powershell
pnpm install --frozen-lockfile
pnpm --filter "@vrecnu/publish" --filter "@vrecnu/desktop-ui" build   # or just: pnpm build
```

```powershell
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings

# ADR-006's shell boundary. Runs in CI; do not delete it.
./scripts/check-shell-boundary.ps1

# Dependency-graph half only, for machines without the SDK.
./scripts/check-shell-boundary.ps1 -SkipBuild
```

These are Cargo commands, **not** `tauri` commands.

## Layout

```
crates/
  vrecnu-types/            domain vocabulary. No platform dependencies.
  vrecnu-shell-api/        THE BOUNDARY: traits, events, AppError.
  vrecnu-capture/          screen capture + the frame pacer.
  vrecnu-audio/            WASAPI loopback + mic, and the silence filler.
  vrecnu-camera/           sole owner of the camera device.
  vrecnu-encode/           the FFmpeg sidecar (child process, never linked).
  vrecnu-namedpipe/        Win32 named pipes feeding audio and camera frames to FFmpeg.
  vrecnu-publish-server/   the loopback server (ADR-002).
  vrecnu-bubble-win32/     the floating camera bubble, raw Win32.
  vrecnu-engine/           orchestration. Owns the threads, knows no shell.
  vrecnu-cli/              headless shell. The swappability proof.
  vrecnu-shell-tauri/      (M6, S1+S2 landed) the ONLY crate allowed to name Tauri.
```

## Status

**M0–M5 complete, all on `main`; M6 S1+S2 (tray shell, TauriShell, read-only IPC, diagnostic screen) done on `main` (merged locally from `feat/m6-tauri-shell`, not yet pushed); tray checks passed 2026-10-15.** On 2026-10-15, `cargo test --workspace` passed **252 tests**
across 12 crates, and `clippy -D warnings`, `fmt --check` and the shell-boundary gate (including
its link-time proof) were clean. The project's status of record is [`HANDOFF.md`](../../HANDOFF.md);
this section summarises it.

Implemented: screen capture and the live encode path, WASAPI audio, the camera and floating
bubble, and the loopback publish server, all driven end to end by `vrecnu-cli`
(`doctor`, `sources`, `cameras`, `record`, `publish`). The engine implements `RecorderEngine`
end to end, publish included. Measured against real hardware for M2–M4; M5 against a real
process, with fake completion POSTs.

Not yet done: the M1 bubble hardware measurements (`TESTING.md` §1), a Drive round trip
(no OAuth client is registered), and the recorder UI (M6 S3+). The Tauri shell exists as a tray
app that opens its window on demand and destroys it on close (S1), and its window shows a live
engine snapshot and device summary (S2 diagnostic screen).

| Milestone | Brings | State |
|---|---|---|
| M1 | The bubble spike. | code complete; hardware measurements pending |
| M2 | Windows Graphics Capture and the live encode path. | done, measured |
| M3 | WASAPI loopback and microphone capture. | done, measured |
| M4 | Camera device and the bubble, integrated. | done, measured |
| M5 | The loopback publish server. | done; on `main` (PR #5) |
| M6 | The Tauri shell and recorder UI. | in progress — S1+S2 done (tray checks passed 2026-10-15), S3 planned (2026-10-15), S4–S5 next |
| M7 | Editing (trim / export). | partial — `render_args` is built and tested, nothing calls it |
| M8 | Installer and auto-update. | blocked on a code-signing certificate |
| M9 | Budget gates. | not started |
