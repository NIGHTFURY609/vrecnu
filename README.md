# vrecnu.

Done. Two documents at the repo root:

HANDOFF.md — where everything stands and what to pick up next:
- Verified state (I ran both suites this turn: Rust 128 tests, TypeScript 135 tests, both exit 0)
- What's finished — Phase 1 complete, ADR-006/007, M0, M1
- Exactly where M2 stopped and the ordered next actions
- M3–M9 scope with the specific traps for each
- The six environment gotchas that each cost real time this session
- Open blockers: signing cert, OAuth client, the RSS budget risk

TESTING.md — the manual checklist, ordered by what matters:
- §1 the bubble spike first — it gates all Phase 2 work, with the 12 go/no-go checks. The two that decide it are "stays above borderless-fullscreen video" and "never steals focus."
- §2 Phase 1, built around the regressions that matter: cancel the screen-share prompt (used to brick the app permanently), watch the JS heap at the moment you press Stop (the §10 violation), trim → seek in VLC, kill the network mid-upload and resume.
- §3 what's checkable in the Rust app today
- §4 an honest list of what isn't testable yet and why

Three things worth flagging for when you resume:

1. M2's WGC backend compiles but has never seen a real screen. Don't trust it until frames actually arrive. Monitor::from_index is 1-based and rejects 0 — noted in the handoff because it's the obvious first trip-up.
2. Nothing is committed. All of this is in the working tree on fet0.1.
3. Registering the OAuth client is the cheapest unblock available. It's a few minutes of clicking, and it unblocks the entire Drive path for both phases. Register all five loopback origins at the same time — ADR-002 is right that it's cheap now and annoying later.
