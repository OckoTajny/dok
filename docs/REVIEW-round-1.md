# Review round 1

Date: 2026-11-05. Reviewed tree: commit `c26d0bc` (before the round-2 fixes).
Three fresh `opencode run` critics were asked to follow `docs/RUBRIC.md`, inspect
only what exists and runs, and return at most ten findings plus a score.

- Product critic (`opencode-go/gpt-6-luna`): completed.
- Engineering critic (`opencode-go/deepseek-v4-pro`): completed.
- UX critic (first attempt, `opencode-go/kimi-k3.1-pro`): timed out after
  15 minutes while waiting on its own background script; no report was
  produced. It was re-run with `opencode-go/gemini-4-argon` and folded into
  round 2 instead of inventing a round-1 result.

## Product / features — 7.5/10

MEDIUM | `src/api.rs:1812-1819`, `src/api.rs:1915-1920` | GitHub webhooks filter for the configured branch, but `deploy_from_repo` clones the repository’s default branch. A webhook for a configured non-default branch can therefore deploy different code than the push that triggered it. | Pass the configured branch into the clone/deploy step and record that branch’s commit; test a non-default branch webhook.

MEDIUM | `src/auth.rs:187-201`, `src/auth.rs:213-219` | Joining an invite only supports new accounts: registration rejects an email that already belongs to a user, and there is no authenticated invite-accept flow. Existing users cannot join another team. | Add an invite-accept flow for signed-in users, with invite validation and membership creation.

MEDIUM | `src/ui.rs:1951-1975`, `src/api.rs:1328`, `src/server.rs:277-289` | The dashboard certificate "Renew" action only writes `cert:<host>` and shows a success message. The ACME worker watches `cert_renew`, which the API renewal endpoint sets, so the dashboard action does not trigger renewal. | Route the dashboard action through the same renewal logic as `POST /api/v1/certificates/{host}/renew`.

MEDIUM | `src/cli.rs:380-409`, `src/api.rs:120`, `src/api.rs:1027-1038` | `dok domain` sets the primary domain on the project record, while `dok domains list/rm` use the separate domains table. A primary domain set by the CLI is not listed there and cannot be removed through that path. | Unify primary and additional domains in one management model, or make list/remove include the primary domain.

MEDIUM | `src/cli.rs:16-75`, `src/api.rs:94-143` | CLI coverage is incomplete for the advertised core: there are no team/member/invite or metrics commands, despite API routes for those operations. | Add CLI commands for team administration, invitations, and project metrics, or narrow the promised CLI parity.

LOW | `src/api.rs:1123-1142` | The metrics response labels the count of at most 100 loaded deployments as `deployments.total`; it also omits development env keys and counts only additional domains. These values can under-report project totals. | Use aggregate database counts and include all supported environments and domain types, or label the values explicitly as recent/partial.

SCORE: 7.5/10

Three blockers:
1. Make configured Git branch deployments deploy the branch that triggered them.
2. Let existing users accept team invites.
3. Make domain, certificate-renewal, metrics and CLI management behaviour consistent across dashboard, API and CLI.

## Engineering / security — 7/10

HIGH | `src/ui.rs:1412-1419` (member "Remove" button) + `src/auth.rs:466` (`esc`) | Stored XSS via the member display name: `esc()` encodes `'` as `&#39;`, but inside a double-quoted HTML attribute the browser decodes it back to `'` before the inline JavaScript runs. A member who registers a crafted name can run script in an owner/admin session. | Escape for the JS string context or use `data-*` + `addEventListener`; never rely on HTML-entity escaping inside inline JS.

HIGH | `src/ui.rs:1077-1146` (env tab) vs `src/api.rs:853-888` (`get_env`) | Environment values are masked for members/viewers by the API but rendered in the dashboard env tab to any reader. | Apply the same write-role masking in the dashboard.

MEDIUM | `src/api.rs:1993-2017`, `src/api.rs:1915-1922` | SSRF through git repo URLs: `valid_repo_url` accepted private and cloud-metadata IP literals, and `deploy_from_repo` then ran `git ls-remote`/`git clone` from the server. | Deny private/loopback/link-local/metadata hosts, add an optional allowlist, and resolve DNS before cloning.

LOW | `src/state.rs:110-119`, `src/proxy.rs:220-256` | Every private/loopback peer was trusted for `X-Forwarded-Proto` by default. | Default-trust only loopback; require `--trusted-proxy` for private ranges.

LOW | `src/auth.rs:417-425` | Logout is a state-changing POST without CSRF. | Require the session CSRF token on logout.

LOW | `src/api.rs:215-221`, `src/ui.rs:1661-1677` | Error responses leaked raw internal error strings (paths, git/docker stderr, SQLite). | Log details server-side and return a generic message.

LOW | `src/proxy.rs:95-112` | `max_body_mb` only inspected `Content-Length`, so chunked requests bypassed the cap. | Enforce the cap while reading the body.

LOW | `src/state.rs:11-27`, `src/auth.rs:86-91` | Rate limiting is per-process and keyed on the socket IP, not the trusted forwarded client IP. | Honour `X-Forwarded-For` from trusted peers and document best-effort semantics.

LOW | `src/db.rs:261-334`, `src/cli.rs:516-523` | Server and post-receive hook can run migrations concurrently on one SQLite file. | Guard `Db::open` with an inter-process lock.

LOW | `src/auth.rs:34`, `src/db.rs` | Fixed 30-day sessions with no rotation or revocation UI. | Add session revocation (or rotate on role changes).

SCORE: 7/10

Three blockers:
1. Fix the stored XSS and audit every inline `onclick`/`confirm`.
2. Apply the API's role masking to the dashboard env tab.
3. Close the repo-URL SSRF surface and narrow default proxy trust.
