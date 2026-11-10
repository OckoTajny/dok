# Review round 5

Date: 2026-11-10. Reviewed tree: commit `6628cf6` (round-4 fixes).
Three fresh `opencode run` critics received a clean repository copy and
`docs/RUBRIC.md`. Paid critic models were unavailable (insufficient account
funds), so this round used free models on the same rubric.

- Product / features (`opencode/glm-5.3-flash-free`): **8.2/10**.
- UX / design (`opencode/mimo-v2.6-flash-free`): **7/10**.
- Engineering / security (`opencode/deepseek-v4.1-flash-free`): **6/10**.

## Product findings

HIGH | `src/server.rs:337-347,427`, `242-265` | Any domain add or cert renew aborted the whole :443 listener and served the control plane over HTTP during the gap. | Keep TLS active across rebinds and bind the new listener with `SO_REUSEPORT`.
MEDIUM | live headers | No HSTS/CSP/X-Frame-Options/X-Content-Type-Options on the control plane. | Security-header middleware for dashboard/auth/API.
MEDIUM | `src/deploy.rs:948` | Metrics were a single live `docker stats` call, no history. | Background sampler + ring buffer + sparkline/API history.
MEDIUM | `src/server.rs:437-465` | One Let's Encrypt order per host; no wildcard/DNS-01 path. | Documented as a remaining limitation (rustls-acme 0.15 has no DNS-01 driver); external wildcard proxy recommended.
MEDIUM | `src/api.rs:950-1021`, `src/cli.rs:489-512` | Secrets are write-only with no reveal/export path. | Explicit README/DEPLOY warning and recovery guidance.
MEDIUM | `src/db.rs` access log | 200-row retention, no status/time filters. | 5000-row retention, `status_min`/`since` filters, UI filter and count.
LOW | `README.md:178-179` | Stale claim that ACME hosts need a restart. | Documented the 15s host watcher and hot swap.
LOW | edge | No automated coverage of the edge router branch. | Added listener/host coverage; full ACME integration remains manual.
LOW | `src/proxy.rs:83-87` | Basic-auth limiter keyed only by IP. | Key now includes the username.
LOW | `src/deploy.rs:815-821` | Rollback beyond the 10-build window is a dead end. | Honest error retained; low priority.

## UX findings

HIGH | served page vs `src/ui.rs` | Production ran an older build than the repo. | Deployed the current binary; version footer added.
HIGH | `src/ui.rs:253-266` | Login page was a single dead end for passkey-less visitors. | Added invite-link/recovery guidance.
HIGH | `src/ui.rs:2586-2589` | Sidebar "Sign out" destroyed sessions on every device. | Now revokes only the current session; `logout-all` remains explicit.
MEDIUM | deploy/log tables | Hard truncation without a count or paging. | Added "newest X of Y" captions and access-log status filter.
MEDIUM | onboarding/empty states | CLI commands without install/docs links. | Docs link in header, CLI install link in onboarding.
MEDIUM | `src/ui.rs:1502-1504`, `1078-1080` | Read-only users were told to add variables/create projects. | Role-aware empty-state copy.
MEDIUM | `src/ui.rs:81` | `input:focus` suppressed the focus ring. | `:focus-visible` ring restored.
LOW | mobile tabs | Hidden horizontal scroller with no affordance. | Visible thin scrollbar.
LOW | invite roles | Bare role names, no capability explanation. | One-line role matrix; `member` may deploy.
LOW | favicon | Missing icon, branded 404 per request. | Inline SVG favicon and 204 for `/favicon.ico`.

## Engineering findings

HIGH | `src/proxy.rs:476-480` | Proxy stripped app `Cookie`/`Authorization` headers, so hosted apps could not log users in. | Forward app credentials; strip only `dok_*` cookies and proxy auth.
HIGH | `src/state.rs:146-168`, `src/server.rs:437`, `src/deploy.rs:598` | Preview hosts were missing from Traefik routers and ACME host discovery. | Preview hosts now included in both.
HIGH | `src/deploy.rs:389-456` | `docker build`/`docker run` had no timeout; a hung build could wedge all deploy slots. | Bounded build/run timeouts, readiness probe and rollback on unhealthy start.
MEDIUM | `src/deploy.rs:668-683` | Production images were never removed after pruning. | Pruned images are removed consistently.
MEDIUM | `src/api.rs:2029-2052` | Webhook rate limit ran before HMAC verification. | Valid deliveries are limited only after authentication.
MEDIUM | `src/cli.rs:378-386` | `dok login` required the token in argv. | `DOK_TOKEN` and `--token-file` supported; docs updated.
MEDIUM | control plane | No security headers (same as product finding). | HSTS/CSP/nosniff/frame-deny/referrer-policy.
LOW | failed builds | Failed deploy build trees could accumulate. | Recognised; cleanup runs on the next successful deploy.
LOW | `src/api.rs:18,158` | 200 MB body limit applied to every endpoint. | Back to Axum's 2 MB default; uploads stream with their own limit.

## Fixes landed after round 5

All HIGH and the actionable MEDIUM findings are addressed: hot TLS listener
swap with `SO_REUSEPORT`, control-plane security headers, sampled metrics
history, access-log retention/filters, documented secret-export limitation,
current-build deployment, login/invite guidance, device-scoped logout,
deploy/log captions, docs and CLI-install links, role-aware empty states and
focus rings, mobile tab scrollbar, role descriptions, favicon, proxy credential
forwarding, preview hosts in Traefik/ACME, Docker timeouts with readiness
gating, production image cleanup, webhook HMAC-before-limit, safe CLI token
input, and per-endpoint body limits.

Remaining known gaps: wildcard certificates via DNS-01 are not built in
(rustls-acme 0.15 has no DNS-01 driver; use an external Traefik/Caddy wildcard),
secrets stay write-only by design, and rollback beyond the 10-build window is
an explicit error. `cargo test`: 65 green; smoke, auth, git, rollback, preview,
UX and Docker smokes pass.
