# Review round 8

Date: 2026-11-10. Reviewed tree: commit `a6b6a9b` (round-7 fixes), live site
updated to that build. Go/free models again.

- Product / features (`opencode-go/mimo-v2.6-flash`): **8/10**.
- UX / design (`opencode-go/mimo-v2.6-flash`): **8/10**.
- Engineering / security (`opencode-go/deepseek-v4.1-flash`): **7.5/10**.

## Product findings

HIGH | repo test suite | `e2e-auth` expected 403 for a bad CSRF token while the UI returned 303 after the round-7 UX change. | CSRF failures are 403 again with friendly copy; `e2e-auth` passes.
MEDIUM | metrics | Only inventory counts + container stats, no traffic analytics despite `access_log` storing status/latency. | Added 1h requests/5xx/p95 traffic to the API, `dok stats` and a dashboard card.
MEDIUM | preview certificates | Preview hosts got ACME orders but were absent from Domains/certificate tables. | Preview hosts are now listed and renew-able; DNS-01 wildcard remains a documented gap.
MEDIUM | non-edge certificates | Traefik-managed instances showed no certificate status. | Clearly labelled "external proxy"; Renew is hidden and the API returns 409.
MEDIUM | quickstart | No published binary; `DOK_BIN_URL` was a placeholder. | Installer supports a prebuilt URL and errors clearly without Rust; publishing releases is release engineering, not repo code.
LOW | HEAD requests | `HEAD /` and `/login` returned 404 while GET returned 200. | HEAD now follows GET in the auth handler.
LOW | webhook oracle | Unknown project names returned 404 before HMAC. | Uniform 401 for unknown/bad signatures.
LOW | docs drift | README said 200 access-log rows, code keeps 5000. | Corrected.

## UX findings

MEDIUM | history | Deploy/log/access history was capped at 20/20/50. | All retained deploys (max ~30) are reachable; access log lists up to 5000.
MEDIUM | blocking deploys | UI redeploy/promote/rollback blocked the request for the whole build. | Spawned in the background and redirect immediately; status poll/live logs carry progress.
MEDIUM | failure copy | Failures were a 300-char CLI-flavoured flash with no log link. | Async actions redirect to the deploy view; flashes keep an ellipsis and friendly copy.
MEDIUM | copy button | `navigator.clipboard` failures were silent. | Clipboard fallback + "Copy failed" feedback.
LOW | 404 copy | Dashboard 404s used app-routing jargon. | Control-host 404 now says "Page not found" with a dashboard back link.
LOW | polled status | Polled badges had no live-region announcement. | `aria-live=polite` on app status; polling does not reload or discard input.
LOW | empty states | CLI-only guidance. | Existing onboarding links the CLI install; repo path is available from Settings.

## Engineering findings

HIGH | proxy body limit | Chunked uploads were buffered in RAM up to 512 MB per request. | Streamed through `http_body_util::Limited`; the ceiling is 64 MB.
HIGH | proxy settings | A DB/parse error silently fell back to default settings (basic auth/force-HTTPS off). | `proxy_settings` returns `Result`; the proxy fails closed with a branded 503.
MEDIUM | renew | "Renew" was a no-op when a valid cached cert existed after the round-7 safety change. | Cache is dropped on explicit renew, but the old listener keeps serving until the replacement cert is deployed (two-phase swap).
MEDIUM | DB hot path | Access-log insert + O(5000) delete on every request behind one mutex. | Retention sweep runs once per 100 writes; settings fault handling is strict.
MEDIUM | container hardening | No pids limit / no-new-privileges. | Both added to `docker run`.
MEDIUM | restart | Deploys stuck in `running` after a crash. | Startup marks them failed/interrupted.
MEDIUM | cert lookup | Expiry could come from an arbitrary stale cache file. | Newest matching certificate wins.
LOW | secret declassify | Blank value + unchecked secret silently declassified a secret. | Blank value keeps the secret classification; a value is required to declassify.
LOW | backups | DB-only backups omitted repos/ACME. | Documented explicitly in the restore instructions.

## Fixes landed after round 8

All HIGH and actionable MEDIUM findings are fixed. `cargo test`: 71 green;
all seven smoke suites plus `e2e-auth` pass. The current build is deployed live.
Known gaps remain: no DNS-01 wildcard certificates, no published prebuilt
release (install builds from source or takes `DOK_BIN_URL`), proxy settings are
read through SQLite on each request, and secrets are plaintext at rest inside a
0600 database.
