# vrecnu

A lightweight, open-source screen recorder that uploads to **your own Google Drive** and hands
back a shareable link. No proprietary cloud, no account with us, no recording bytes touching
any server we run.

Four invariants govern every decision: **lightweight**, **a no-install path always exists**
(the PWA), **bring-your-own-storage**, **privacy**.



---

## Where the project stands

A summary — the detailed, dated status of record is [`HANDOFF.md`](HANDOFF.md).

| Phase | What | Status |
|---|---|---|
| **1** | Web app (PWA) — record, trim, download, upload to Drive | code complete — not yet run against real Google Drive, not deployed |
| **2** | Installed desktop app (Rust core, Tauri shell) | in progress — see below |
| **3** | Full desktop editor | not started |

**Phase 2 milestones:**

| M | Scope | Status |
|---|---|---|
| M0 | Rust workspace (10 crates then, 11 now), the shell boundary and its CI gate | done |
| M1 | Floating camera bubble (raw Win32 layered window) | code complete; needs a human at a real screen (`TESTING.md` §1) |
| M2 | Screen capture → FFmpeg → seekable MP4 | **video path done and measured** |
| M3 | System audio + microphone | **audio path done and measured** |
| M4 | Camera + floating bubble | **camera path done and measured** |
| M5 | Loopback publish server | **done, exercised against a real process** — on `main`; never run against real Drive |
| M6–M9 | Tauri shell + UI, editing, packaging, budgets | not started (M7 partial: `render_args` is built and tested, nothing calls it yet) |

Try the recorder as it stands:

```powershell
cd apps/desktop
$env:VRECNU_FFMPEG = "C:\path\to\ffmpeg.exe"   # M8 will bundle one
cargo run --release -p vrecnu-cli -- doctor
cargo run --release -p vrecnu-cli -- sources
cargo run --release -p vrecnu-cli -- record --seconds 30 --out clip.mp4
```

`vrecnu-cli` is not a toy: it exists to prove the engine has no shell dependency, and CI builds
it on every commit for exactly that reason.

---

## Where to read next

| Document | For |
|---|---|
| `ARCHITECTURE.md` | what the system is and why |
| `AGENTS.md` | the canonical spec — tech-stack choices, security rules, the "do NOT" list |
| `docs/adr/` | why each rule exists; change one with a **new** ADR, not an edit |
| `read_this_idiot.md` | the Tauri escape hatch and how not to break it |
| `HANDOFF.md` | current state, next actions, and the environment traps that cost real time |
| `TESTING.md` | what a human has to verify by hand, ordered by what matters |
| `CLAUDE.md` | working notes for Claude-based agents |

Read them in that order. When `AGENTS.md` and anything else disagree, **`AGENTS.md` wins.**

---

## Known blockers

- **No OAuth client registered.** Blocks every real Drive round-trip, on both phases. It is
  the cheapest unblock available — a few minutes of clicking. Register a **Web application**
  client with the `drive.file` scope *only*, and add all five loopback origins
  `http://127.0.0.1:47821` … `:47825` at the same time (ADR-002: cheap now, annoying later).
- **No code-signing certificate.** Blocks M8 packaging (ADR-005 action item 5).
- **The ≤150 MB idle RSS budget is at risk with Tauri.** ADR-006 says to measure in M9 and
  amend with real numbers attached — and to reopen the ADR rather than reflexively raise the
  budget if the tray mitigation is not enough.

## License

MIT. FFmpeg is invoked as a child process and never linked — see
[ADR-004](docs/adr/004-ffmpeg-sidecar-and-size-budgets.md) for why that distinction is
load-bearing rather than stylistic.
