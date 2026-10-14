# Registering the Google OAuth client

**Status: not done.** No OAuth client exists for vrecnu yet, which is why the record → Drive →
link loop has never been executed end to end. This document is the runbook: every value is
pre-computed from the repository so registration is clicking, not deciding.

Two audiences:

- **§1–§4** — the maintainer registering the project's own client.
- **§6** — a self-hosting user registering *their own* (the bring-your-own-credentials path
  `AGENTS.md` §5 requires, the same escape hatch rclone offers).

Everything here needs a Google account and a browser. Nothing in it can be scripted, and
nothing in it should be — an OAuth client is a credential, and the person who owns the account
should be the person who creates it.

---

## 1. What to create, exactly

| Field | Value | Why it must be this |
|---|---|---|
| Application type | **Web application** | *Not* "Desktop app". A desktop client would push you toward the native loopback-**redirect** flow, which `AGENTS.md` §5 explicitly forbids. Both the PWA and the desktop publish page are browser contexts running the same GIS token client, so both are web clients. |
| Scope | `https://www.googleapis.com/auth/drive.file` **and nothing else** | Non-sensitive, touches only files vrecnu creates, and avoids Google's restricted-scope security assessment entirely. Any broader scope breaks the privacy promise *and* puts the project into an audit process. |
| Client secret | **Not used.** Ignore the one Google generates | The GIS token client is a public-client flow: no secret, no refresh token, ~1-hour access tokens. (It is deliberately *not* authorization-code + PKCE — Google requires the client secret at the token endpoint for "Web application" clients, so a secretless PKCE flow isn't available to a static page.) `AGENTS.md` §5: no client secret in the repo or any shipped bundle. Google issues one for web clients regardless; leave it in the console and never copy it anywhere. |

## 2. Authorized JavaScript origins

Register **all seven** in one sitting: **five** desktop publish-page origins, **one** local
dev origin, and **one** deployed-PWA origin. Adding origins later is a console round-trip plus
propagation delay, and the five loopback ports are needed the moment the desktop publish flow
is exercised (the loopback publish server, M5, is implemented — [ADR-002](adr/002-desktop-publish-loopback-origin.md)
action item 3: cheap now, annoying later).

```
http://127.0.0.1:47821
http://127.0.0.1:47822
http://127.0.0.1:47823
http://127.0.0.1:47824
http://127.0.0.1:47825
http://localhost:5173
https://<your-pages-domain>
```

- **Desktop publish page (5): the five `127.0.0.1` ports** are the desktop publish server's, chosen in that order
  ([ADR-002](adr/002-desktop-publish-loopback-origin.md)); they are `REGISTERED_PORTS` in
  `apps/desktop/crates/vrecnu-publish-server/src/lib.rs` and the two lists must not drift.
  Loopback is the *only* exemption Google grants from its https-origins rule — which is the
  whole reason the publish page is served over HTTP from a port rather than from a
  `vrecnu://` custom scheme. A custom scheme is not a registrable origin and fails with
  `origin_mismatch`.
- **PWA / publish-page dev origin (1): `http://localhost:5173`** is the Vite dev server, for
  working on `packages/publish` and `packages/web` locally. It is for development only and is
  not used by any shipped build. Neither `vite.config.ts` pins a port, so this is Vite's default —
  and running both dev servers at once pushes the second to **5174**, which is a *different
  origin* to Google and fails with `origin_mismatch`. Register `http://localhost:5174` too, or
  pin the port in the config; whichever you pick, the console and the config have to agree.
- **Deployed PWA origin (1): `https://<your-pages-domain>`** is wherever the Phase 1 PWA is
  hosted — the only `https://` entry in the list. Not yet chosen — the PWA is not deployed anywhere, so add this when
  it is.

> Use `127.0.0.1`, not `localhost`, for the five desktop ports: the server's `Host`-header
> check accepts both, but Google treats them as distinct origins and the desktop navigates to
> the numeric form.

**Authorized redirect URIs: leave empty.** The GIS token client in a browser does not use a redirect URI.
If you find yourself filling this in, the flow has drifted to the native loopback-redirect one
that `AGENTS.md` §5 forbids — stop and re-read it.

## 3. OAuth consent screen

| Field | Value |
|---|---|
| User type | **External** |
| Publishing status | **Testing** is fine to start — it allows up to 100 test users, which is plenty to prove the loop |
| Scopes | Add `.../auth/drive.file` only |
| Test users | Your own Google account |

Moving to **In production** later requires Google's verification for the consent screen, but
**not** a security assessment — that is exactly what staying inside `drive.file` buys.

## 4. Where the client id goes

The id is public (it ships in the bundle by design; the *secret* is what must never ship).

```bash
# .env at the repo root, not committed -- .env.example documents the variable.
VITE_GOOGLE_CLIENT_ID=<the id>.apps.googleusercontent.com
```

`packages/publish/src/credentials.ts` resolves it, and a user-supplied id in `localStorage`
deliberately **beats** this build-time default so a self-hoster can override a hosted build
without rebuilding it. That resolution order is the bring-your-own-credentials path, and it is
already implemented and unit-tested.

## 5. Proving it works

In order, because each step's failure mode is distinct:

1. `pnpm --filter @vrecnu/publish dev`, open the page, click connect. (The first token
   request must come from that click — a user gesture — or the browser's popup blocker
   suppresses Google's window.) **Expect** Google's consent screen naming exactly one permission: *"See, edit, create and delete only the
   specific Google Drive files you use with this app."* Any wider wording means the scope is
   wrong — stop and fix it before uploading anything.
2. Record something short in the web app and publish it. **Expect** a `webViewLink` on the
   clipboard, and the file visible in *your* Drive.
3. Open that link in a private window. **Expect** it to play — that proves the
   `permissions.create {role:"reader", type:"anyone"}` step, which is easy to get wrong in a
   way only a second identity reveals.
4. Upload something large enough to interrupt (a few hundred MB), kill the network mid-upload,
   restore it. **Expect** the upload to resume rather than restart. `AGENTS.md` §5 treats a
   non-resumable upload as a defect, not an edge case, and the resumable path is currently
   tested only against a fake Drive.

None of this has been done. Step 1 is the one that catches a wrong scope before any real data
moves, so do not skip it.

## 6. Bring your own credentials (self-hosters)

You do not need the project's client — vrecnu is designed so you never have to trust it with
one. Create your own Google Cloud project and follow §1–§3 above, registering **the same five
loopback origins** if you intend to use the desktop app. Then paste your client id into the
publish page's credentials panel; it is stored in `localStorage` and overrides whatever the
build shipped with.

This is the rclone model, and it removes the project as a bottleneck: no user cap, no
dependency on anyone else's verification status, and no need to trust a third party's OAuth
client with access to files in your own Drive.

## 7. What is deliberately *not* here

- **No client secret**, anywhere. See §1.
- **No service account.** Recordings go to the *user's* Drive under the user's own
  authorization; a service account would put them in an account vrecnu controls, which
  inverts the entire premise.
- **No server-side token exchange**, because there is no server. `AGENTS.md` §5: recording
  bytes go browser → the user's Drive and must never transit vrecnu infrastructure. Anything
  the project hosts is static.
