# Review round 7

Date: 2026-11-10. Reviewed tree: commit `bb9cd79` (round-6 fixes), live site
updated to that build. Paid critic quota was exhausted; this round used Go/free
models.

- Product / features (`opencode-go/mimo-v2.6-flash`): **7.5/10**.
- UX / design (`opencode-go/mimo-v2.6-flash`): **7.5/10**.
- Engineering / security (`opencode-go/deepseek-v4.1-flash`): **7/10**.

## Product findings

BLOCKER | `install.sh`, units | Installer defaulted to the author's domain, hardcoded an external Traefik, and a user unit could not bind :80/:443. | `DOMAIN`/`BASE_DOMAIN` required; `SELF_EDGE=1` installs the edge, ACME args and `setcap`; localhost is always a dashboard alias.
HIGH | `src/deploy.rs` readiness | TCP accept reported success before HTTP answered, so the next request could 502. | HTTP readiness probe (non-5xx) before cutover.
HIGH | `src/proxy.rs` force HTTPS | Scheme was derived from the public URL, so `force_https` did nothing behind HTTPS termination. | Scheme comes from `--edge` or trusted `X-Forwarded-Proto`; regression covered.
HIGH | dashboard role gate | Members saw Promote/Rollback/Redeploy but the dashboard POST rejected them. | Deploy actions use `can_deploy`; settings still use `can_write`.
HIGH | ACME renew | Renew deleted the shared cached certificate before a replacement existed. | Cache is kept; the old certificate serves until ACME deploys a new one.
MEDIUM | certificate UI | Non-edge instances showed pending certs and a no-op Renew. | External-proxy status, no Renew, API returns 409.
MEDIUM | `hosts_for_app` | Custom-domain projects still got a cert/ACME order for an unrouted default host. | Default host only when no custom domain.
MEDIUM | passkeys | No way to add a backup passkey. | Account → Add passkey flow with WebAuthn preflight.
MEDIUM | admin role API | UI allowed admin member/viewer role changes, API/CLI required owner. | `can_grant` is the single rule for UI/API/CLI.
MEDIUM | previews | Up to ten preview containers stayed running forever. | Preview restart policy `no`, max three running previews per project.

## UX findings

HIGH | auto-refresh | Full-page reload every 5s discarded input and live streams. | In-place status polling via `/dashboard/status`; no reload.
HIGH | session message | First-time `/dashboard` visitors saw a false "session expired" alert. | Expired copy only when a session cookie was actually present.
MEDIUM | passkey errors | Raw browser exceptions on insecure origins. | `isSecureContext`/`PublicKeyCredential` preflight with actionable copy.
MEDIUM | access log | Every HTTP status rendered as the same gray badge. | Status-aware `http_badge` with 2xx/3xx/4xx/5xx classes.
MEDIUM | mobile tables | `width:100%` defeated the scroll wrappers. | `table{min-width:640px}` restores horizontal scrolling.
MEDIUM | stale CSRF | Redirected to the project list and lost the form. | Redirects to the referring dashboard URL with the flash.
MEDIUM | project cards | Card hover promised a click that only the title link delivered. | Whole card is a single link; duplicate footer links removed.
LOW | account/team labels | Account heading was inert; "API tokens" hid the page. | "Account & API tokens"; read-only action column explains itself.
LOW | empty/action copy | Read-only deploy rows had a blank action column. | Explicit read-only label.

## Engineering findings

HIGH | `src/cli.rs` pack | `dok push` followed symlinks, uploading files outside the project. | Symlinks are skipped; regression test.
HIGH | invite table | Viewers saw and could copy active invite tokens. | Invite table gated on `can_write`.
MEDIUM | SSRF | IPv4-mapped/6to4 IPv6 literals bypassed the private-address check. | `to_ipv4_mapped`, 6to4/Teredo rejection; tests added.
MEDIUM | upload/delete race | Delete during upload could resurrect a teamless project. | `require_existing` under the deploy lock.
MEDIUM | upload pressure | 200 MB writes and 500 MB extracts were unbounded. | Upload semaphore and pre-write free-space check.
MEDIUM | branch XSS | Hook branches were not escaped in the logs picker; `deploy_git` did not validate refs. | Escaped at render and validated in `deploy_git`.
MEDIUM | member dashboard | Same as product finding. | Fixed with `can_deploy`.
MEDIUM | deploy errors | Input failures returned 500. | 413/422/409 with the underlying message; build dirs cleaned.
LOW | git hook routers | `deploy-git` had no Traefik dir. | Path persisted and loaded.
LOW | secrets at rest | Plaintext SQLite/backups. | Threat model documented.

## Fixes landed after round 7

Every BLOCKER/HIGH and all actionable MEDIUM findings above were fixed.
`cargo test`: 71 green. All seven smoke suites pass. The build is deployed
live. Remaining known gaps: DNS-01 wildcard certificates are not implemented
(rustls-acme 0.15 has no DNS-01 driver); secrets are plaintext at rest and
write-only by design; retrying a deploy older than the retained 10 builds is an
explicit error.
