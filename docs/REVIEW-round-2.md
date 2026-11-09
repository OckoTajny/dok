# Review round 2

Date: 2026-11-09. Reviewed tree: commit `b9ff0d9` (the round-2 handoff commit).
Three fresh `opencode run` critics were given only a clean copy of the repo and
`docs/RUBRIC.md`; they had no context from the working session. Each ran the
unit tests locally; the UX critic also tried the browser suite (Playwright was
not installed in its sandbox), then continued with source review.

- Product / features (`opencode-go/gpt-6-luna`): **7/10**.
- UX / design (`opencode-go/gemini-4-argon`): **7.5/10**.
- Engineering / security (`opencode-go/deepseek-v4-pro`): **8/10**.

## Product / features — 7/10

HIGH | `src/server.rs:174-182` | Edge mode with no domains at startup fell back to plain HTTP and returned before starting the host watcher, so adding a domain later could not enable TLS without a restart. | Keep the edge supervisor running; serve the dashboard over HTTP until TLS is ready.
HIGH | `src/api.rs:2052` | Git and webhook deployments always deployed production, so a pushed branch could not produce a preview. | Support branch/PR preview deployments with their own URLs and an explicit production branch.
MEDIUM | `src/deploy.rs:131-136` | Every auto-detected Node project used one Dockerfile that ran `npm start` and never built Vite/Next-style projects. | Add framework-aware build/run defaults and actionable errors.
MEDIUM | `src/cli.rs:16-89` | CLI had no proxy, certificate or access-log commands although the API exposes them. | Add CLI parity commands.
MEDIUM | `src/api.rs:792-845` | Env writes only accepted strings; the secret flag was dashboard-only. | Accept `{value, secret}` objects and a `--secret` CLI flag.
MEDIUM | `src/proxy.rs:696-716` | Compression only handled static gzip, not Brotli or proxied responses. | Compress static and proxied text with gzip/Brotli negotiation.

## UX / design — 7.5/10

HIGH | `src/ui.rs:763-768`, `src/auth.rs:363-392` | No dashboard/API token self-service: invited teammates could sign in but had no way to get a token for `dok login` / `dok push`. | Add personal API token create/copy/revoke UI, API endpoints and CLI commands.
HIGH | `src/ui.rs:960-963`, `1045-1046`, `1324-1344`, `1378-1420` | Mutating dashboard/domain/proxy/settings forms were shown to read-only `member`/`viewer` users, dead-ending in 403/422. | Gate every mutating form on `can_write`, with a read-only notice.
HIGH | `src/ui.rs:2521-2540` | Settings could not clear `repo`/`framework` (upsert used `COALESCE`) and did not validate repo URLs. | Explicit `UPDATE`, validate `valid_repo_url`, allow clearing.
MEDIUM | `src/ui.rs:479-495` | Validation errors replaced the whole dashboard shell with a standalone 422 page and lost input. | Redirect back with a flash error inside the normal layout.
MEDIUM | `src/ui.rs:2475-2487` | Invalid or reserved proxy headers were silently dropped while flashing success. | Validate and reject with a clear message.
MEDIUM | `src/ui.rs:1412-1416` | Webhook URL, secret and branch were API-only; no dashboard or CLI surface. | Add Settings form and `dok webhook`.
MEDIUM | `src/ui.rs:54-110`, `789-799` | Mobile `.btn.link` targets stayed 22 px, and Escape did not reset `aria-expanded` or restore focus. | Fix the media rule and drawer focus/ARIA handling.
MEDIUM | `src/ui.rs:1116-1186`, `1481` | Logs tab showed raw epoch timestamps, dead runtime copy/download buttons on static apps, irrelevant Docker stats, and unlinked deploy ids. | Format timestamps, hide dead controls, link deploy ids and badge the live deploy.
LOW | `src/ui.rs:1687-1740` | Invite refresh re-posted, the caption contradicted the copy button, and an existing member could consume an invite. | Copy button in the notice, corrected caption, membership check before consuming.
LOW | `src/ui.rs:54-62`, `152-158` | Disabled WebAuthn buttons kept hover styles; raw browser errors were shown. | Disabled style, busy label and friendly NotAllowed/Abort mapping.
LOW | `src/ui.rs:905-936`, `1435` | Duplicate failure badges, inconsistent deploy-id lengths and no live badge. | Single failure badge, 8-char ids, live badge.

## Engineering / security — 8/10

MEDIUM | `src/api.rs:754-857` | Env secret flag was unreachable from API/CLI. | Per-key secret objects, `secrets` list, `dok env --secret`.
MEDIUM | `src/db.rs:265-338` | Migration errors were swallowed while `user_version` advanced. | Propagate ALTER errors except duplicate-column, with a failure-path test.
MEDIUM | `.github/workflows/ci.yml:15-16` | E2E suites were manual-only. | CI jobs for smoke, Docker and Playwright/passkey suites.
MEDIUM | `src/api.rs:168-190`, `546-562`, `src/state.rs:11-27` | Authenticated API had no rate limiting or concurrency/disk quota. | Per-user rate limits, deploy/stream semaphores, free-disk guard.
LOW | `src/api.rs:465-528` | First-upload project creation could race ahead of the deploy lock. | Atomic `INSERT OR IGNORE` plus a write check for the loser.
LOW | `src/api.rs:650-692` | `docker logs -f` streams had no concurrency cap. | Bounded stream semaphore.
LOW | `src/db.rs:1308-1312` | Backup `VACUUM INTO` ran on the shared connection and stalled requests. | Dedicated backup connection.
LOW | `src/api.rs:792-857` | Env keys were validated only in the dashboard. | Validate keys centrally in the API path.

## Fixes landed after round 2

All HIGH/MEDIUM findings above were addressed before round 3:

- Edge supervisor now stays alive with an empty domain set; HTTP serves the
  dashboard until TLS is ready and switches automatically (`src/server.rs`).
- Git/webhook branch deployments produce previews for non-production branches;
  `dok git-init` hooks pass the pushed ref (`src/api.rs`, `src/cli.rs`).
- `deploy_dir` preserves an existing primary domain and repo across plain
  pushes, and the dashboard can clear repo/framework explicitly.
- Node auto-detection builds Vite/Astro/React/Svelte/Nuxt output or runs
  `npm start` when a start script exists (`src/deploy.rs`).
- API/CLI: secret env objects + `dok env --secret`, `dok proxy`, `dok certs`,
  `dok access-log`, `dok webhook`, `dok tokens`.
- Proxy compression negotiates Brotli/gzip for static and upstream responses;
  invalid/reserved headers are rejected (`src/proxy.rs`, `src/api.rs`).
- Personal API tokens: Account page, create/copy/revoke, `/api/v1/tokens`,
  `dok tokens`, plus "Sign out everywhere".
- Every mutating dashboard form is role-gated; validation redirects back with
  an inline flash instead of replacing the shell.
- Webhook settings are in the Settings tab, invite copy/PRG-ish behavior and
  existing-member handling are fixed, mobile drawer focus/ARIA and touch
  targets are fixed, and the Logs tab is cleaned up.
- Migrations fail loudly; CI runs smoke, Docker and browser suites; uploads
  are rate-limited and bounded by deploy/stream semaphores and free disk.
