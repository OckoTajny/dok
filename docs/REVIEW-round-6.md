# Review round 6

Date: 2026-11-10. Reviewed tree: commit `f361676` (round-5 fixes); the live
site was also updated to that build before the review.
Critics used free/Go models after paid critic quota ran out.

- Product / features (`opencode-go/longcat-2.5-free`): **8/10**.
- UX / design (`opencode-go/mimo-v2.6-flash`): **8/10**.
- Engineering / security (`opencode-go/deepseek-v4.1-flash`): **7/10**.

## Product findings

HIGH | `src/api.rs` github_hook | Any webhook event without `refs/heads/*` (including GitHub's ping) fell through to a production deploy. | Ignore non-branch deliveries with 204; added test.
HIGH | `src/api.rs` set_repo | `dok git` sent `{"repo"}` but the API required `{"domain"}`, so the documented CLI always returned 422. | `RepoBody` accepts both; CLI verified live.
HIGH | `src/proxy.rs` serve_app/log_request | Access log and basic-auth limiter used the socket peer (Traefik IP) instead of the visitor. | Trusted `X-Forwarded-For` client IP; `--trusted-proxy` added to units; live row now shows the real IP.
MEDIUM | `README.md` proxy claim | README said Cookies/Authorization are stripped although app auth is forwarded. | Corrected; `authorization` added to response-header denylist.
MEDIUM | delete paths | Project deletion stopped containers but leaked Docker images. | Collect and remove images on both API and UI delete.
MEDIUM | `src/deploy.rs` promote | Promote/rollback skipped readiness and lost commit metadata. | Readiness gate + metadata copy.
MEDIUM | env file | Multiline env values were silently flattened. | Rejected with a clear error; API save rejects newlines.
MEDIUM | first-run setup | Claim committed before user/team creation, so a failure could brick setup. | One atomic `create_first_setup` transaction.
MEDIUM | `install.sh`, units | Source-only install and no trusted proxy in units. | `DOK_BIN_URL` prebuilt path and trusted-proxy in shipped units.
LOW | `git-init` | Literal `<user>@<host>` remote and branch not set on rerun. | Real path/ssh remote and branch always ensured.

## UX findings

HIGH | zero-team state | A user removed from their only team got a dashboard that referenced a nonexistent team and a 404 on `/dashboard/team`. | Dedicated "No team yet" create-team page on both routes; tested.
MEDIUM | public 404 | Branded 404 had no clickable way back. | Back button to the dashboard.
MEDIUM | error meta | Many pages printed `Request id -`. | Placeholder omitted.
MEDIUM | stale CSRF | A rotated session produced a full 403 "invalid CSRF token" page. | Redirect + flash "session changed"; test updated.
MEDIUM | status freshness | Deploy status never refreshed. | Auto-reload while a deploy badge is `running`.
LOW | session expiry | Silent redirect to login. | Flash "Your session expired".
LOW | team highlight | Sidebar never marked the selected team outside `/dashboard`. | Selected team highlighted on project/account pages.
LOW | 502 retry | `javascript:` anchor. | Real button.
LOW | auth landmarks | Auth pages had no `<main>`. | Added.

## Engineering findings

BLOCKER | `src/deploy.rs` dok.toml dockerfile | Absolute/`..` Dockerfile paths could read host files and echo them into logs. | Canonicalised relative-path check; regression test.
HIGH | ACME default | Edge used Let's Encrypt staging by default. | Production is now the default; explicit `--acme-staging`; old flag accepted as no-op.
HIGH | preview env | Preview containers inherited production secrets and were public. | Secrets are no longer inherited into preview/development; only explicit per-env secrets are injected.
HIGH | basic auth | Argon2 ran before the limiter and the limiter map could grow. | Pre-verification IP+user block, failure-only hits, bounded/evicted buckets.
MEDIUM | cert renew UI | Arbitrary hostnames could trigger global ACME renewal. | Host must belong to the project.
MEDIUM | member domain | A deploy query could change the primary domain as a member. | Domain changes require write rights; `dok.toml` domain only applies on first deploy.
MEDIUM | local repo | `<data>/repos/../victim.git` passed the lexical prefix check. | Canonicalise and require the project's exact bare repo.
MEDIUM | proxy client IP | Same as product finding. | Fixed with trusted forwarded IP.
MEDIUM | delete/deploy race | Delete could be undone by a concurrent deploy. | Delete takes the per-app deploy lock and rechecks the row.
MEDIUM | deploy errors | All deploy failures became opaque 500s and left build trees. | 422 with the real message and build-dir cleanup.

## Fixes landed after round 6

All BLOCKER/HIGH and every actionable MEDIUM finding above were fixed.
`cargo test`: 70 green. Smoke, auth, git, rollback, preview, UX and Docker
smokes pass. The current build is deployed live with `--trusted-proxy`.

Remaining known gaps: wildcard certificates via DNS-01 are not built in
(rustls-acme 0.15 has no DNS-01 driver); secrets remain write-only by design;
rollback beyond the retained 10 builds is an explicit error.
