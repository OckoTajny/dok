# Review round 4

Date: 2026-11-09. Reviewed tree: commit `b155640` (round-3 fixes).
Fresh critics, same rubric, clean repo copies.

- Product / features (`opencode-go/gpt-6-luna`): **8/10**.
- UX / design (`opencode-go/gemini-4-argon`): **8/10**.
- Engineering / security (`opencode-go/deepseek-v4-pro`): **8/10**.

## Product findings

MEDIUM | `src/cli.rs:160-164,683-688` | `dok proxy set` could enable compression/force HTTPS but not disable them. | Add `--no-compression` and `--no-force-https`.
MEDIUM | `src/cli.rs:177-181,396-402` | `dok projects create` could not choose a team. | Add `--team` and pass it through.

## UX findings

HIGH | `src/ui.rs:835-841`, `146-147` | New browser users had no prompt telling them to mint a CLI token before the empty state's `dok push` command. | Add an onboarding callout with a direct Account → API tokens link.
MEDIUM | `src/ui.rs:1474-1476,1547-1683` | Read-only roles saw red `.error` banners and no Proxy/Settings values. | Neutral read-only cards with the current values.
MEDIUM | `src/ui.rs:1148-1179` | Stored ACME error/note was not rendered on the Domains tab. | Show the error/note under the certificate host.
MEDIUM | `src/ui.rs:1488-1544` | Domains tab could not promote a custom domain to primary; `public_url_for` ignored custom domains when `--base-domain` was unset. | Add Make primary; fall back to the first custom domain.
MEDIUM | `src/ui.rs:1638-1663` | `git-init` stores a local repo path that Settings then rejected on save, and the built-in git flow was undiscoverable. | Allow local `repos/<app>.git` paths and show a git-init hint.
LOW | `src/ui.rs:1537` | Domains table lacked `<thead>`. | Added.
LOW | `src/ui.rs:66-120` | Mobile toggle/member-row touch targets and overflow. | (Low; partially addressed by existing 44px rules.)
LOW | `src/ui.rs:1203-1358` | Logs tab could pair the latest preview build log with the production runtime container; selector did not auto-submit. | Resolve the latest deploy when no env is chosen and auto-submit.
LOW | `src/ui.rs:836` | Sign out was an `<a href="#">` fetch. | Semantic POST form.

## Engineering findings

MEDIUM | `src/db.rs:245-252`, `383-416`, `1457-1464`, `src/server.rs:543-573` | Secret values in SQLite/WAL/backups were created with the caller's umask (often 0644). | Set process umask 077, data/backup dirs 0700, DB/WAL/SHM/backups 0600.
LOW | `src/api.rs:209-228`, `1260` | Domain claim was check-then-insert; a race returned 500. | `INSERT OR IGNORE` + 409/clean message.
LOW | `src/state.rs:11-27` | Rate-limiter map never evicted empty buckets. | Drop empty buckets and sweep above a cap.
LOW | `src/api.rs:2436-2458` | SSH repo URLs were not DNS-pinned. | HTTPS is pinned with `http.curloptResolve`; SSH remains protected by host-key checks (documented residual).
LOW | `src/auth.rs:305-307` | Anonymous email enumeration during registration. | Accepted self-hosted UX tradeoff; not changed.
LOW | `src/api.rs:1966-1979` | Git hook built with unescaped shell interpolation. | Shell-quote every interpolated path.
LOW | `src/api.rs:2016-2134` | Webhook to a missing project returned 500. | Return 404.

## Fixes landed after round 4

All HIGH/MEDIUM findings were fixed before the final verification pass:
API/CLI proxy disable flags and project `--team`, onboarding token callout,
neutral read-only Proxy/Settings cards, certificate error/note rendering,
custom-domain primary promotion + URL fallback, local git repo paths/hint,
domains table header, latest-deploy log consistency and auto-submit, semantic
logout, private file permissions for the database/WAL/SHM/backups, atomic
domain insert with a clean conflict, bounded rate-limiter map, quoted git hooks
and webhook 404s.
