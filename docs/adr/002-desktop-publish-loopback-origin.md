# ADR-002: Desktop publish page is served from a loopback HTTP origin

**Status:** Accepted
**Date:** 2026-08-02
**Deciders:** Maintainer (Jeswin Christie)
**Relates to:** [ADR-001](001-phase-2-pure-rust-desktop.md)

## Context

Under [ADR-001](001-phase-2-pure-rust-desktop.md), the Phase 2 desktop app is pure Rust and
**does not implement Google Drive OAuth or upload at all**. Publishing is delegated to a web
view that loads `packages/publish` — the same "connect & upload" page the Phase 1 PWA uses —
so there is exactly one PKCE implementation to maintain.

`ARCHITECTURE.md` §4a originally left the page's origin as an "open sub-decision" and
**recommended bundling the page inside the exe and serving it through a `wry` custom URI
scheme** (e.g. `vrecnu://…`), on the grounds that a self-contained page fits the portability
invariant better than depending on our hosting being up.

**That recommendation cannot work.** Google requires every entry in a web OAuth client's
**Authorized JavaScript Origins** to use the `https://` scheme, with loopback origins
(`http://localhost`, `http://127.0.0.1`) as the *only* exemption. A custom URI scheme is not
a registrable origin. A page served from `vrecnu://` would fail the GIS token request with
`origin_mismatch`, and the entire Phase 2 publish path would be dead on arrival.

There is a second, independent problem the original design carried: the page's origin and
the local recording's origin were different, which forced a CORS-restriction rule
(`AGENTS.md` §5) and a cross-origin fetch of a large video file.

Constraints that must survive whatever we choose:

- No client secret anywhere (public client, PKCE / GIS token client).
- Scope is `drive.file` only.
- Recording bytes go **browser → user's Drive**, never through a vrecnu server.
- The local file endpoint must expose **only** the current recording, loopback-only,
  one-time-token-gated, torn down after publish.

## Decision

The Phase 2 desktop app **bundles the publish page's static assets inside the exe and serves
them over a loopback HTTP server on `127.0.0.1`**, from a **fixed port chosen from a small
registered list**. The web view (or the system browser) is pointed at that origin.

**The same loopback server, on the same origin, also serves the current recording.** The
publish page therefore fetches the video **same-origin** — no CORS involved.

Registered ports, tried in ascending order until one binds:

```
47821, 47822, 47823, 47824, 47825
```

All five must be registered in the Google Cloud OAuth client as Authorized JavaScript
Origins (`http://127.0.0.1:47821` … `:47825`) and as redirect origins where applicable. If
all five are occupied, publishing fails with an actionable error — it must **not** fall back
to an unregistered ephemeral port, because that produces a confusing Google error screen
rather than a vrecnu one.

The `wry` custom-URI-scheme option is **removed**, not deferred.

## Options Considered

### Option A: Bundled page via `wry` custom URI scheme (the previous recommendation)

| Dimension | Assessment |
|---|---|
| Complexity | Low |
| Self-contained | Excellent |
| **Works at all** | **No** |

**Pros:** no TCP port at all; fully offline-capable asset serving; smallest attack surface.
**Cons:** **fatal** — a custom scheme cannot be an Authorized JavaScript Origin, so Google
OAuth fails with `origin_mismatch`. Additionally, the file bridge would be cross-origin
relative to any hosted page.

### Option B: Hosted page (load `https://<our-domain>/publish` in the web view)

| Dimension | Assessment |
|---|---|
| Complexity | Low |
| Self-contained | **Poor** — depends on our hosting |
| OAuth compatibility | Excellent (a normal https origin) |
| Version skew | None — always current |

**Pros:** trivially satisfies Google's origin rules; page and web app never drift; no local
HTTP server needed for the page itself.
**Cons:** the portable exe stops being self-contained — if our static host is down or
blocked, publishing breaks even though the user's own Drive is reachable. The recording file
still needs a loopback endpoint, and that fetch is now **cross-origin**, requiring the CORS
dance the original design specified. Also means a shipped exe is silently coupled to
whatever we deploy later — a version-skew hazard in the other direction.

### Option C: Bundled page served over loopback HTTP, fixed port (chosen)

| Dimension | Assessment |
|---|---|
| Complexity | Medium — a small HTTP server, port negotiation |
| Self-contained | Excellent — assets ship in the exe |
| OAuth compatibility | Good — loopback is Google's documented exemption |
| Security surface | Small, and *smaller than the alternatives* (same-origin) |

**Pros:** satisfies Google's origin rules via the loopback exemption; assets are pinned to a
known-good build inside the exe, so no dependency on our hosting and no skew; **page and
recording share one origin**, which deletes the CORS requirement and collapses two local
endpoints into one.
**Cons:** requires binding a TCP port (a real, if small, local attack surface); the port must
be pre-registered with Google, so it cannot be fully ephemeral; a port collision is a
user-visible failure mode; the bundled page can go stale relative to the PWA and must be
refreshed at release time.

## Trade-off Analysis

Option A is eliminated on correctness, not preference.

Between B and C, the deciding factor is the **portability invariant**. A portable recorder
whose publish step breaks when our CDN has a bad day is not meaningfully portable, and
"bring your own storage" loses much of its point if the client still has a hard runtime
dependency on infrastructure we operate. Option C keeps the only network dependency the one
that is *inherently* required: the user's own Google Drive.

The security objection to C — "you opened a port" — is weaker than it looks, and C is in
fact the *safer* of the two live options. Under B, the recording must be served from a
loopback endpoint anyway, and that endpoint has to be deliberately opened up with CORS to
let a remote origin read it. Under C there is one loopback origin, no cross-origin grant,
and the file fetch is same-origin by construction. **C removes a security control by
removing the need for it.** Same-origin also means the capability token never has to survive
a cross-origin request.

The genuine cost of C is the **fixed-port registration**. It couples the desktop build to
the Google Cloud OAuth client configuration: the port list becomes part of the app's public
contract and cannot be changed without re-registering. This is accepted, and the list is
deliberately five ports wide so collisions are rare. It also means users on the
bring-your-own-credentials path must register the same origins in their own Google Cloud
project — this must be documented in the BYO-credentials setup guide.

## Consequences

**Easier:**

- Phase 2 publishing works at all, which was not true before.
- **The CORS rule in `AGENTS.md` §5 is obsolete** — the file fetch is same-origin. One fewer
  control to get right, and one fewer to get wrong.
- One loopback server instead of a server *and* a custom-scheme handler.
- The exe is self-contained: no dependency on vrecnu hosting for the publish path.

**Harder:**

- The desktop app must ship a small HTTP server and negotiate a port. The remaining
  hardening rules stay fully in force: serve **only** the current recording and the bundled
  publish assets, bind to `127.0.0.1` only, gate the recording with a **one-time capability
  token**, and shut the server down when publishing completes.
- The port list is now part of the OAuth client configuration and the BYO-credentials docs.
- The bundled publish page must be refreshed at each desktop release; add a build check that
  the bundled `packages/publish` hash matches the intended release build.
- All-ports-occupied is a real failure mode needing a clear error message.

**To revisit:**

- If Google ever changes the loopback origin exemption, this ADR must be reopened
  immediately — there is no fallback that preserves both invariants.
- If a hosted page later becomes preferable (e.g. for hotfixing the upload path without
  shipping a new exe), that is a new ADR superseding this one, and it must reintroduce the
  CORS rule it deletes.

## Action Items

1. [x] Delete the custom-URI-scheme publish option from `ARCHITECTURE.md` §4a; record the
       loopback-origin decision there.
2. [x] Update `AGENTS.md` §5: replace the CORS-restriction bullet with the same-origin model;
       keep single-file, loopback-only, one-time-token, teardown-after-publish.
3. [ ] Register `http://127.0.0.1:47821` … `:47825` as Authorized JavaScript Origins on the
       shared OAuth client.
4. [ ] Document the same port registration in the bring-your-own-credentials guide.
5. [ ] Implement port negotiation with an explicit, user-readable "all ports busy" error.
6. [ ] Add a release check that the bundled publish assets match the intended build.
