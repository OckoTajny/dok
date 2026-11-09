# Review round 3

Date: 2026-11-09. Reviewed tree: commit `96d6d52` (round-2 fixes).
Three fresh `opencode run` critics again received only a clean repository copy and
`docs/RUBRIC.md`.

- Product / features (`opencode-go/gpt-6-luna`): **7/10**.
- UX / design (`opencode-go/gemini-4-argon`): **8.3/10**.
- Engineering / security (`opencode-go/deepseek-v4-pro`): **7/10**.

## Product findings

HIGH | `src/cli.rs:609-655`, `src/api.rs:1341-1387` | `dok proxy set` read redacted settings and wrote them back, silently disabling basic auth by saving a user without its hash. | Preserve the stored hash unless a new password is supplied; reject a user without any password on first setup.
MEDIUM | `src/server.rs:294-307,424-433` | Cert renewal bumped one global trigger and cleared/reissued every certificate. | Track the requested host, clear only that host's cache, and reissue only it.
MEDIUM | `src/deploy.rs:414-416`, `src/api.rs:640-651` | Preview containers have deploy-specific names while the log stream defaulted to `dok-app-preview`; CLI could not pick a deploy. | Resolve the selected/latest deploy's recorded container and add `dok logs --deploy`.
MEDIUM | `src/cli.rs:801-808` | `dok open` built the default URL from the CLI server host instead of the configured base domain. | Return and use the project's resolved public URL.
LOW | `src/cli.rs:88-95` | No CLI team create/role/remove commands. | Add them.

## UX findings

HIGH | `src/ui.rs:1994`, `2405-2420` | `POST /dashboard/team/invite` rendered 200 from the POST URL, so refresh duplicated invites and GET was a 404 dead end. | Short-lived invite cookie + 303 redirect to the team page.
HIGH | `src/ui.rs:1469-1470`, `2769-2778` | Saving a basic-auth user with no password/hash flashed success while auth stayed disabled. | Reject the form/API update with a clear message.
HIGH | `src/ui.rs:1863-1905`, `2467-2517` | Member rows offered invalid controls: last-owner removal/demotion, and admins could demote owners. | Hide impossible controls and require owner for owner/admin changes.
MEDIUM | `src/ui.rs:753-790`, `2085`, `2875` | Sidebar active state wrong on Team/Account; `?team=` dropped by account links and redirects. | Active-section state, team context preserved.
MEDIUM | `src/ui.rs:1288-1381` | Preview/development env views hid inherited production variables. | Show inherited rows with an `inherited` badge.
MEDIUM | `src/ui.rs:1615-1645` | Deployment rows attributed git/webhook deploys to `token` and never showed the branch. | Use `commit_author`/`author` and render the branch.
MEDIUM | `src/ui.rs:2336-2339` | Sign-out-everywhere flash was never consumed on the login page and leaked into the next session. | Render and clear flash on the login page.
MEDIUM | `src/ui.rs:1590-1663`, `2563-2595` | Long synchronous deploy actions had no busy state and a misleading "started" message. | Disable/label the submit button and report completion.
LOW | `src/ui.rs:79-119` | Missing `.inline`, mobile tab underline, and `word-break:break-all` prose. | CSS fixes.
LOW | `src/ui.rs:891-916`, `1267-1274`, `2056-2071` | "Open app" dead link before first deploy, duplicated live log tail, malformed bulk env lines silently ignored. | Hide the link, clear the log before streaming, validate lines.

## Engineering findings

HIGH | `src/db.rs:811-833`, `src/api.rs:324-348`, `src/ui.rs:2088-2143` | Project creation checked existence then used the overwriting upsert; a concurrent create from another team could take ownership and reset routing state. | Use `create_app_if_absent` in both paths, then re-read and verify write access for the loser.
MEDIUM | `src/db.rs:679-685`, `src/api.rs:1646-1684` | Last-owner protection was check-then-act; concurrent demotions could leave zero owners. | Transactional `*_checked` member methods that refuse the last owner.
MEDIUM | `src/db.rs:896-923` | Deleting a project left webhook secrets/branch settings and the bare repo/hook behind, so a recreated name inherited them. | Delete webhook settings and `repos/<app>.git`.
MEDIUM | `src/api.rs:2388-2421` | DNS was checked once but git resolved again at connect time (rebinding TOCTOU). | Return the checked addresses and pin them via `http.curloptResolve` for HTTPS.
LOW | `src/api.rs:1579-1592` | Team slug creation is check-then-insert and can 500 on a race. | (Still open; low.)
LOW | `src/state.rs:11-27` | Rate limiter is per-process and basic-auth limiting keys on the proxy IP. | (Still open; low, documented single-instance assumption.)
LOW | `src/api.rs:2047-2051` | Webhook branch was not charset-validated before git argv. | Validate `[A-Za-z0-9/_.-]` and reject leading `-`.
LOW | `src/ui.rs:553-598` | Flash/token cookies omit `Secure` even on HTTPS. | (Still open; low.)

## Fixes landed after round 3

- Project creation uses the atomic insert in API, dashboard and `git-init`.
- Member role/remove use transactional last-owner guards; impossible controls are
  hidden and admins cannot change owner/admin roles.
- Project deletion removes webhook settings and the bare repo.
- HTTPS repo fetches pin the checked DNS addresses with `http.curloptResolve`.
- Invite creation is PRG with a one-time cookie; invite notice has Copy.
- Proxy basic auth rejects a user without any password and preserves the hash on
  unrelated updates.
- Cert renewal tracks the requested host and clears only that cache entry.
- Log streams resolve preview containers; CLI can select `--deploy`; `dok open`
  uses the API's resolved URL; team create/role/remove exist in the CLI.
- Sidebar active state and `?team=` context, inherited env rows, commit
  author/branch, login flash, busy states, CSS polish, pre-deploy links, live
  log clearing and strict bulk env parsing were fixed.
