# CLAUDE.md — vrecnu

Guidance for Claude Code (and Claude-based agents) working in this repo.

## Read this first

**`AGENTS.md` is the canonical spec. Read it before doing anything non-trivial.** It holds
the full project overview, the fixed tech-stack choices, the security rules, and the
"do NOT" list. This file does **not** repeat all of that — it adds Claude-specific working
notes and re-states only the rules that are easiest to violate. If this file and `AGENTS.md`
ever disagree, **`AGENTS.md` wins.**

Also read `ARCHITECTURE.md` for *what/why*. Order of reading: `ARCHITECTURE.md` →
`AGENTS.md` → this file.

## One-paragraph context

**vrecnu** is a lightweight, open-source, no-install screen recorder that uploads to the
user's **own Google Drive** (not a proprietary cloud) and hands back a shareable link.
Four invariants govern every decision: **lightweight, portable/no-install,
bring-your-own-storage, privacy.** Delivery is phased: Phase 1 web app (PWA) → Phase 2
portable **pure-Rust** `.exe` (see AGENTS.md ADR-001) → Phase 3 full desktop editor. Do not
build later-phase features into an earlier phase.

## How to work in this repo (Claude specifics)

- **Plan before large changes.** For anything spanning multiple files or a new subsystem,
  use plan mode / lay out the approach and get agreement before editing.
- **Prefer the dedicated tools** (Read/Edit/Grep/Glob) over shell `cat`/`sed`/`find`.
- **Use subagents for verification** on non-trivial work: after implementing, spawn a
  reviewer to check against the constraints in `AGENTS.md` (scope rules, package boundaries,
  no committed secrets) before you report done.
- **Keep a task list** for multi-step work so progress is visible.
- **Verify, don't assume.** Run `pnpm typecheck` + `pnpm lint`, and for recorder/upload
  changes confirm an actual record→file→playable (and, where wired, upload→link) round-trip.
  The repo is **pre-scaffold** today — do not run build commands against an empty repo;
  update `AGENTS.md` §8 when the scaffold lands.
- **This is a public open-source repo.** Assume everything you write is world-readable.
  No secrets, no internal URLs, no personal data in code, comments, or commits.

## Environment / MCP notes

This workspace may expose extra MCP connectors (e.g. Supabase, n8n, browser automation).
**vrecnu Phase 1–2 needs none of them** — it is a client-side PWA + a pure-Rust desktop app
with no backend and no database. Do **not** introduce Supabase/a database/a server-side component
into the architecture just because a connector is available; that would violate the
"thin client / bring-your-own-storage" invariant. If a backend ever seems necessary, stop
and raise it with the maintainer first — it is an architecture change, not an implementation
detail.

If a Google Drive or browser connector is genuinely useful for *testing* the real API
during development, that's fine — but shipped code still follows `AGENTS.md` §5 exactly.

## The rules most likely to trip you up (re-stated — full list in AGENTS.md §10)

- **pnpm only** (TS workspace). Never `npm install` / `yarn`. Desktop is Cargo/Rust.
- **Phase 2 is a pure-Rust native app — not Tauri, not Electron** (AGENTS.md ADR-001). A web
  view (`wry`) or the system browser is used **only** for the Drive publish step.
- **Google Drive scope = `drive.file` and nothing else.** Never a broader/restricted scope;
  never read the user's existing files.
- **Drive is web/TS only.** The Rust app never implements OAuth/upload — it hands the local
  recording to the publish web view, which does PKCE + browser→Drive upload (AGENTS.md §5).
- **No secrets committed. No client secret in any bundle** (use GIS/PKCE in the web/publish
  page for both phases; no native loopback-redirect OAuth).
- **Uploads must be resumable. Recording bytes never transit a vrecnu server.**
- **`packages/core` and `packages/storage` stay platform-agnostic** — no browser/Node
  imports. Drive access only through the `StorageProvider` interface.
- **No FFmpeg-heavy editing in the Phase 1 web app** (trim only). Deep editing is Phase 2.
- **No content telemetry, ever.**

When a task appears to require breaking one of these, **stop and ask** rather than working
around it.

## Commits

Conventional-commit style (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`). Branch off
`main`; don't push to `main` unless told. Keep commits scoped; update `ARCHITECTURE.md` and
these guidance files when a decision changes.
