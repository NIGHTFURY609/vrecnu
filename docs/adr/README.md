# Architecture Decision Records

Each file here records **one** architectural decision: the situation that forced it, the
options weighed, what was chosen, and what it costs. ADRs are **immutable once accepted** —
when a decision changes, write a *new* ADR that supersedes the old one rather than editing
history.

**Relationship to the other docs:**

- `ARCHITECTURE.md` — what we are building and why (the narrative).
- `AGENTS.md` — the rules you must follow when building it (the constraints).
- `docs/adr/` — **why each rule exists** (the reasoning).

If a rule in `AGENTS.md` looks arbitrary, the ADR is where the reason lives. Do not "fix"
a constraint without reading its ADR first — most of them exist to avoid a bug we already
found.

## Format

Filename: `NNN-kebab-case-title.md`. Numbers are sequential and never reused.

Status is one of: **Proposed** | **Accepted** | **Deprecated** | **Superseded by ADR-NNN**.

**Amendments.** A decision whose *reasoning* changes but whose *conclusion* stands gets an
amendment block at the top of the original file pointing at the newer ADR (see ADR-001).
A decision whose conclusion changes gets a **new** ADR that supersedes the old one. Never
silently rewrite an accepted ADR — the wrong-turn record is the point.

## Index

| # | Title | Status | Date |
|---|---|---|---|
| [001](001-phase-2-pure-rust-desktop.md) | Phase 2 is a pure-Rust desktop app, not Tauri | Accepted — rationale amended by 005 | 2026-08 |
| [002](002-desktop-publish-loopback-origin.md) | Desktop publish page is served from a loopback HTTP origin | Accepted | 2026-08-02 |
| [003](003-phase-1-capture-pipeline.md) | Phase 1 capture pipeline: chunk-to-disk, container choice, composite clock | Accepted — §3.1 amended 2026-08-03 (OPFS) | 2026-08-02 |
| [004](004-ffmpeg-sidecar-and-size-budgets.md) | FFmpeg ships as a separate process; numeric size/CPU budgets | Accepted — reaffirmed 2026-08-03 | 2026-08-02 |
| [005](005-installed-app-not-portable.md) | Phase 2 ships as an installed app, not a portable exe | Accepted | 2026-08-03 |
