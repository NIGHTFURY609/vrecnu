# vrecnu desktop (Phase 2)

The Rust side of vrecnu. See [ADR-006](../../docs/adr/006-phase-2-tauri-shell-with-swappable-core.md)
for the shell decision and [`read_this_idiot.md`](../../read_this_idiot.md) for the escape
hatch back to a pure-Rust GUI.

## Prerequisites

Rust stable (1.89 or newer) **and the Windows SDK**.

> **The Windows SDK is not optional and its absence is confusing.** Without it, `cargo build`
> — and even `cargo check` — fails with:
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
  vrecnu-publish-server/   the loopback server (ADR-002).
  vrecnu-bubble-win32/     the floating camera bubble, raw Win32.
  vrecnu-engine/           orchestration. Owns the threads, knows no shell.
  vrecnu-cli/              headless shell. The swappability proof.
  vrecnu-shell-tauri/      (M6) the ONLY crate allowed to name Tauri.
```

## Status

**M0 complete.** 114 tests pass, `clippy -D warnings` is clean, `fmt --check` is clean, and
the shell-boundary gate passes including its link-time proof.

Implemented: the workspace, the boundary traits, the domain types, and the pure
logic each subsystem is built around — the frame pacer, the buffer pool, the audio silence
filler, FFmpeg argv construction and `-progress` parsing, HTTP Range parsing, the capability
token, the session state machine, and the error model.

Not yet implemented: the platform backends themselves. `vrecnu-cli` reports which milestone
owns each missing piece rather than pretending to succeed:

| Milestone | Brings |
|---|---|
| M1 | The bubble spike. **Gates everything after it.** |
| M2 | Windows Graphics Capture and the live encode path. |
| M3 | WASAPI loopback and microphone capture. |
| M4 | Camera device and the bubble, integrated. |
| M5 | The loopback publish server. |
| M6 | The Tauri shell and recorder UI. |
