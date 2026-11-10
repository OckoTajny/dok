# dok product audit — what the dashboard is for

Date: 2026-11-10 · Worktree: `~/projects/dok-product`, branch `design/product`
Scope: README, all of `docs/`, `src/{ui,api,cli,deploy,db}.rs`, live site
`https://dok.solta.tech`, plus a local instance on `:18093` with a fresh
database, four projects (one failed, one preview), a custom domain and env
vars. No product code was changed.

## 0. Verdict

dok is feature-rich but the dashboard is still a **project admin panel**, not a
place where work happens. It answers "which projects exist?" and then hands the
user a form. A real person opens dok with a different question: *what is live,
what changed, what is broken, and what do I do next?* The current home screen
answers none of those.

The owner's "empty and small" is structural, not cosmetic:

- The global page is a card grid of 260px-minimum tiles above a lot of black
  space (`src/ui.rs:57`, `src/ui.rs:1133-1222`, `src/ui.rs:1262-1266`). With
  four projects it is two short rows; most of the viewport is empty.
- Every card is one line of metadata and a badge. There is no aggregate, no
  timeline, no health, no action other than "open project".
- The primary global action is an inline `new-project` input
  (`src/ui.rs:1250-1258`). It looks like the main thing to do, even for a team
  with 40 projects.
- The product's most valuable facts are buried one project deep: deploy
  outcomes, certificate state, access-log errors, environment configuration.

The biggest risk is **trust**, not emptiness:

1. A preview deploy can replace the live state on the project card, because the
   card renders the newest deploy of any environment
   (`src/ui.rs:1135-1136`, `src/ui.rs:1183-1187`). `hello` showed
   `preview · just now · 022f11ae` while production `13565d13` was live.
2. A failed app is a red badge with no reason. The failure log exists but the
   overview never surfaces it (`src/ui.rs:2118-2127`, `src/ui.rs:2146-2169`).
3. The branded error page shows a request id for support
   (`src/ui.rs:425-441`) but the access log never stores it
   (`src/db.rs:270-280`), so the operator cannot look it up.
4. A project with zero successful deploys still reports "Runtime: static files
   · served from disk" (`src/ui.rs:2118-2127`), which is simply false.

That is why the dashboard "feels small": it reports inventory instead of
state, and some of the state it reports is misleading. The fix is not a visual
refresh; it is deciding that dok's home is an **operations view of the team's
fleet** and that each project page tells the story of the last deploy.

## 1. Personas & jobs

| Persona | Arrives to… | Success looks like | What blocks or confuses them today |
|---|---|---|---|
| **Solo dev shipping a side project** (primary) | Push code and get a URL; later find out why it broke | `dok push` → a link they can share; a failed deploy they can fix without SSH | Token and CLI setup are scattered; the dashboard shows counts, not the last deploy; failures have no reason; preview can mask production; domains need undocumented DNS work |
| **Small team (2-6)** | Know what is live and who changed it; roll back; invite someone; manage env | One screen that answers "what changed, what is broken, what is safe to ship"; safe rollback; env separation | No team-wide activity or audit; production and preview are mixed; no notifications; roles are one sentence; no passkey inventory; invites are copy-link only |
| **Self-hoster / admin** | Keep the instance healthy: certs, backups, disk, Docker, upgrades | Cert expiry, backup age, disk/queue health, version and upgrade status visible before something breaks | There is no instance page at all; certificate state lives per project; no backup/disk/version/Docker health; no outbound alerts; no dok logs |

### A. Solo dev shipping a side project

Job story: *"I have 40 minutes after dinner. I want to ship my side project,
send the link to a friend, and not think about servers."*

A realistic session today:

1. Registers a passkey on first visit (`src/ui.rs:259-275`) and lands on a team
   page whose main content is a `new-project` input (`src/ui.rs:1262-1266`).
2. Creates `my-app`, sees the flash "Push to it with `dok push --app my-app`"
   but no token, no login command, and no working directory
   (`src/ui.rs:1250-1258`; observed locally).
3. Has to discover **Account & API tokens** in the sidebar
   (`src/ui.rs:937-945`), create a token, copy it, then assemble
   `dok login <url> <token>` by hand. The one-time notice prints the command
   (`src/ui.rs:2253-2262`) but only the token has a copy button.
4. Runs `dok push`. The server responds with a build and a URL; the CLI output
   is good (`src/cli.rs:1935-1940`, `src/cli.rs:1987-1992`). If the user
   switched back to the browser, only the status badge updates on the 5-second
   poll (`src/ui.rs:986-990`); the rest of the card is stale until reload.
5. Opens the project: five equal cards, three of which are counts of zero
   (`src/ui.rs:2146-2169`). Nothing says "your site is live at this URL, from
   commit X, 12 seconds ago".
6. Pushes a preview (`dok push --env preview`). The **app card now shows the
   preview as the app's state** — `preview · just now · 022f11ae` — while the
   real production deploy `13565d13` is only a small badge
   (`src/ui.rs:1135-1136`, `src/ui.rs:1183-1187`, `src/ui.rs:1213-1214`).
7. A bad deploy (`broken` in the local run) yields a red `failed` badge with no
   reason. The only paths to the error are opening the Logs tab (it defaults
   to the latest deploy's log) or noticing that the deploy id is a link in the
   history table (`src/ui.rs:2072-2083`).
8. Adds a custom domain. The UI has an input and a certificate table, but no
   DNS target, no verification, and no warning that making it primary stops
   routing on the default hostname (`src/ui.rs:1814-1837`,
   `README.md:277-279`).

What this persona needs is not more settings. It is **confirmation**: a green
"live" line, the URL, the commit, the time, and on failure the first actionable
error line with a link to the full log.

### B. Small team (2-6)

Job story: *"It's Monday, the client says the site is down, and I need to know
whether someone deployed, what changed, and how to get back."*

Today the team gets a project list and per-project pages, but no shared memory:

- **No team-wide activity.** The only global view of project state is a card per project
  (`src/ui.rs:1113-1289`). "What shipped in the last 24 hours?" requires opening
  every project and reading its deployment table.
- **No audit.** Changing an env var, adding a domain, or changing a role writes
  nothing except the current row. The schema has no events/audit table
  (`src/db.rs:178-281`); the user sees no "who changed what".
- **Production and preview are mixed in one table**, sorted only by time
  (`src/db.rs:1111-1138`; rendered `src/ui.rs:2009-2084`). The row for a preview
  is visually identical to production except for the env badge.
- **Roles are explained in one line**: "owner: everything… member: deploy and
  read · viewer: read-only" (`src/ui.rs:2476`). The actual rule is in
  `src/db.rs:110-122`; "member: deploy" is true, but nothing on screen explains
  that a member cannot edit env, domains, or settings.
- **Notifications do not exist.** The only webhook is *inbound* GitHub
  (`src/api.rs:172`); no failure mail, chat message, or outbound webhook.
- **Passkeys can only be added**, never listed or removed
  (`src/ui.rs:2271-2274`; auth routes only start/finish,
  `src/auth.rs:188-193`). A lost laptop cannot be de-authorized from the UI.
- **Invites are a copy-link ritual**, valid seven days, single use
  (`src/ui.rs:2394-2452`, `src/ui.rs:2471-2479`); there is no email invite, no
  pending-invite badge on the dashboard, and no per-project access.

Success for this persona is a **team pulse**: last deploys across projects,
failures, who did what, and one-click rollback. dok already stores almost all of
it; it just never assembles it.

### C. Self-hoster / admin

Job story: *"I run this for my team on a VPS. I want to know it will still be
healthy next month."*

There is no page that answers this. The dashboard routes are team, project,
account only (`src/ui.rs:2704-2848`); the sidebar has no system section
(`src/ui.rs:883-953`). Concretely missing:

- Certificate inventory and expiry warnings at team level. Certificates are
  rendered per project in the Domains tab (`src/ui.rs:1319-1391`); an expiring
  cert on a project nobody opens is invisible.
- Backup age/status. Backups are written by the server
  (`src/server.rs`/`src/state.rs`), but the UI never shows the last backup.
- Disk usage, build-cache size, retained builds, Docker availability, queue.
- The version (`src/ui.rs:947-950`) is shown, but not whether an upgrade exists
  or what changed.
- No `dok` service logs or health history; Docker stats failures surface only
  as "docker stats unavailable" in `dok stats` (`deploy.rs:1169`) or "n/a" in
  the dashboard (`src/ui.rs:2118-2127`).

This persona is the most likely to file a "why did my dashboard feel empty"
complaint that is actually about **operational confidence**, not project count.

## 2. Journey map

Friction marked with evidence. "UI" is `src/ui.rs` unless stated.

| # | Step | What the person does | What dok shows | Friction |
|---|---|---|---|---|
| 1 | First run | Opens the dashboard URL, creates a passkey, email, name, optional team | A passkey setup page then an empty team page | No system check, no "instance is healthy", no next steps. The optional team field silently becomes "Personal" (`UI:259-275`, `auth.rs:318`, `auth.rs:497-498`). |
| 2 | Install CLI | Finds `install.sh` in the README | Header link "Docs" → GitHub README (`UI:974`); `/install.sh` and `/docs` return 404 on a running instance (verified on `:18093`) | The product tells the user to leave the product to install it. No copyable curl command, no self-hosted docs route. |
| 3 | Token | Opens Account & API tokens, creates a token | Token shown once, copy button, `dok login <url> <token>` text | No copy-command button (`UI:2253-2262`), no token last-used, no scopes, no name validation. The notice disappears on reload. |
| 4 | First deploy | Creates a project, runs `dok push` | CLI is good; the dashboard badge catches up on the 5s poll, but the rest of the card does not | Create flash omits the directory and token (`UI:1250-1258`); only the badge is polled (`UI:986-990`, `UI:2739-2759`), so host/time/deploy-id text stays stale until reload; no build progress. |
| 5 | Failure | Deploy fails | Red `failed` badge on card and history | No error excerpt, no phase, no "retry", no prominent link. Project page still says "static files · served from disk" (`UI:2118-2127`). |
| 6 | Custom domain | Adds `app.example.com` | Domain table, default-host line, certificate table | No DNS target or verification (`UI:1814-1837`); copy says "default hostname **below**" while the hostname is above (`UI:1815` vs `UI:1827-1831`); making the domain primary silently drops the default host (`README.md:277-279`, `proxy.rs:429-434`). |
| 7 | Env/secrets | Adds production keys, marks secrets | Per-key row editor; inherited values in non-prod | One form per key with Save/Remove (`UI:1659-1680`); inherited rows have no "override" action (`UI:1712-1717`); bulk editor hidden behind a details disclosure (`UI:1745-1750`); no `.env` import and no timestamps. |
| 8 | Git/webhook | Wants `git push` or GitHub deploys | Repo URL, branch, write-only secret, webhook URL | Two different mental models (repo URL vs `dok git-init`) in one form (`UI:1915-1918`, `UI:1993-2003`); no connection status, no test delivery, no last webhook result. |
| 9 | Preview | `dok push --env preview`, shares the URL | Preview row and URL | No previews section; the preview can take over the project card (`UI:1135-1136`, `UI:1183-1187`); promote is a text link in one row (`UI:2019-2026`); no cleanup/TTL UI. |
| 10 | Rollback | Clicks "Roll back here" on a production row | Browser confirm, then a new build | No indication that production stays up during the build, no "why this deploy", no post-rollback confirmation beyond the badge (`UI:2027-2042`). |
| 11 | Invite teammate | Creates an invite on Team page | Copy-link notice and invite table | Link-only, 7 days, single use (`UI:2394-2452`); no email; no pending-invite state on the dashboard; role selector needs the explanatory line below it (`UI:2471-2476`). |
| 12 | Day-2 ops | Checks certs, backups, disk, failures | Nothing global | No instance page, no alerts, no audit, no search. Certificates only inside each project (`UI:1319-1391`); request ids on error pages are not searchable (`UI:425-441` vs `db.rs:270-280`). |

### The three sharpest journey failures

**Failure is a dead end.** The CLI gets a clear 422
("cannot detect a build…", observed) and the deploy log contains the reason,
but the dashboard renders only `failed`. The user must infer that the short
deploy id in the history table is a link (`UI:2072-2083`), then read the build
log. On the dashboard card there is not even that. And the Overview's "Runtime"
card actively misleads by claiming static files are served when no successful
deploy exists (`UI:2118-2127`).

**Production truth is not the default.** The card loop takes the latest deploy
across environments (`UI:1135-1136`) and prints its env, age and id
(`UI:1183-1187`). In the local run, `hello` had a live production deploy
`13565d13`, but the card said `preview · just now · 022f11ae`. The `prod` badge
was still there, so the card showed two competing states. This is the single
most confusing thing on the home screen.

**Error pages cannot be used for support.** Branded 404/502/504 pages include a
request id and "last good deploy" (`UI:409-441`), which is excellent. But
`access_log` stores only app/time/method/path/status/latency/ip
(`db.rs:270-280`, insert at `db.rs:1688-1700`). There is no request-id column
and no search box. The operator literally cannot resolve the id the page gave
the visitor.

## 3. What is missing

Ranked by user value first, effort second. Effort is for this codebase: **S**
under a day, **M** 1–3 days, **L** more than 3 days.

| # | Missing capability | Why it matters | Evidence today | Value | Effort |
|---|---|---|---|---|---|
| 1 | **Team activity feed** (last deploys/changes across projects) | Answers "what changed?" without opening every project; makes an empty dashboard feel alive | Home only lists project cards (`UI:1113-1289`); deploys are per project (`UI:2009-2084`) | High | S–M |
| 2 | **Fleet health summary + filters** | One glance: how many live, failed, building, previews; filter the grid | No counts; only cards (`UI:1262-1266`) | High | S |
| 3 | **Failure reason + next action** on card/overview | Turns a red badge into a fix | Log exists, never surfaced (`UI:2146-2169`, `deploy.rs:501-515`) | High | S |
| 4 | **Deploy progress with phases and duration** | Removes the silent wait after `dok push`; tells whether it is uploading, building, starting, live | Only `running`/`ok`/`failed` (`deploy.rs:389`, `deploy.rs:755`); schema has `created_at` only (`db.rs:206-212`) | High | M |
| 5 | **Previews workspace** (group by branch, open, promote, clean up) | Previews are a core feature but invisible as a set | Mixed into one table (`UI:2009-2084`); no cleanup UI | High | M |
| 6 | **Domains & TLS table with DNS target and verification** | Custom domains have the most external dependencies (DNS, certs) and no in-product guidance | Input + cert table only (`UI:1814-1837`) | High | M |
| 7 | **Instance page** (certs, backups, disk, Docker, version, upgrade) | Self-hosters need confidence before failure | No route/section (`UI:883-953`, `UI:2704-2848`) | High | M |
| 8 | **Outbound notifications** on failed deploy / expiring cert | Day-2 ops cannot watch the dashboard all day | Only inbound GitHub webhook (`api.rs:172`) | High | M |
| 9 | **Request-id search / error → access log link** | Makes branded errors actionable | Request id shown (`UI:425-441`), not stored (`db.rs:270-280`) | Medium | S |
| 10 | **Project settings**: rename, move team, port, memory, CPU | These exist in `dok.toml`/CLI flags only; the dashboard hides them | Settings edits repo/framework only (`UI:1915-1918`, `UI:3477-3486`) | Medium | S–M |
| 11 | **Passkey inventory and removal** | Security hygiene; lost device | Add only (`UI:2271-2274`) | Medium | S |
| 12 | **Access-log search** (path, method, time range) | Debugging beyond status filter | Status-only filter (`UI:1484-1542`, `db.rs:1708-1737`) | Medium | S |
| 13 | **Restart / stop app** | The first thing a dev tries when an app hangs | Deploy only; no route in `api.rs:84-172` | Medium | S |
| 14 | **Audit log** (env/domain/role/config changes) | Teams need accountability | No events table (`db.rs:178-281`) | Medium | M |
| 15 | **Cross-project search / command palette** | Navigation once there are >10 projects | Sidebar flat list (`UI:916-931`) | Medium | S |
| 16 | **Traffic chart** (requests, 5xx, p95 over time) | The data exists; the card is one line | 1h aggregate only (`UI:2134-2145`) | Medium | M |
| 17 | **Per-project permissions** | Teams outgrow team-wide roles | Team-level only (`db.rs:110-131`) | Low–Med | L |
| 18 | **Production approval gate** / protected envs | Prevents accidents | Anyone with deploy role can ship (`db.rs:120-122`) | Low–Med | L |
| 19 | **Git provider OAuth / app install** | Removes manual webhook setup | Manual URL + secret (`UI:1948-1962`) | Low | L |
| 20 | **DNS-01 wildcard certificates** | Many previews | Known gap (`docs/REVIEW-round-8.md` closing paragraph) | Low–Med | L |

### Notes on the top gaps

- **Activity feed** can be built from existing data: `deploys` has app, env,
  status, author, commit, branch, created_at (`db.rs:206-212`, `db.rs:1111-1138`).
  No schema change is needed for a first version.
- **Progress** does need a schema change (`started_at`/`finished_at`, or a phase
  field) and log-derived phase markers. Until then, at least show elapsed time
  from `created_at` and a spinner; the current badge never changes until the
  whole build ends.
- **Notifications** can reuse the existing per-project webhook secret idea but
  need an outbound destination model. Start with a generic outbound webhook URL
  and a failure-only default; email is a bigger lift.
- **Instance health** should be a read-only page, not a settings page. Show
  facts; only backup retention/upgrade actions need controls.

## 4. What is unnecessarily complex

### 4.1 Dashboard: a create form is the hero

Current: team name, subtitle, "Team & invites", then a full-width `new-project`
input + button, then "All projects" and cards (`UI:1262-1266`).

Why it is wrong: the primary action on a team page should be *using* the team,
not adding another project. With projects present, the form still consumes the
top third of the page. With none, it duplicates the empty state's advice.

Simpler: one primary `New project` button in the page header (opens a small
dialog or `/dashboard/new`), project count/health in the header, and the grid
starting immediately. The empty state owns the first-project instructions.

### 4.2 Project header: badge soup

Current: `hello` + hostname, then `Open app` + green `ok` + `prod 13565d13`
(`UI:2194-2204`). The deploy id is truncated to 8 characters with no label and
no link.

Simpler: one state line — `Live · deploy 13565d13 · 2m ago` — with the deploy id
linking to its log. `Open app` stays. Drop the redundant `ok` badge or merge it
into the state line.

### 4.3 Overview: five equal cards, three of them vanity

Current: Deployments total, Domains, Environment keys, Traffic (1h), Runtime
(`UI:2146-2169`). "Domains 0 · custom + primary" and "Environment keys 2" do not
change what the user does next. "Traffic 16 req · 0.0% 5xx · p95 0 ms" is a
sentence pretending to be a metric.

Simpler: a **health card** (Live / Failed / Building with the last deploy and
time), a **traffic card** with a real 24h sparkline and error count, and a
**runtime card** with CPU/memory sparkline. Move counts into small links:
`2 deployments · 1 domain · 2 env keys` under the header.

### 4.4 Deployments: the deploy id is the primary key of the user's mental model

Current table: Deploy (id + author + commit + URL), Env, Status, Deployed,
Actions (`UI:2009-2084`). Production and preview rows are interleaved by time.
The "Deployed" column prints relative age and absolute UTC time on every row
(`UI:738-804`, `UI:2072-2083`).

Simpler: group under **Production** (one pinned "live" row: status, commit,
author, age, actions) and **Previews** (collapsible, count, branch, open,
promote, delete). Columns become: state, what (commit/message), who, when.
Put the deploy id in a tooltip or secondary line, not the first column.

### 4.5 Logs: three tools stacked in one tab

Current: deploy picker, build log, runtime log, stats, then "Recent requests",
status filter and a 100-row access table (`UI:1593-1619`). The copy says
"Newest 2 deploys are selectable here; older builds remain available through the
API" (`UI:1595`) — API jargon in a dashboard.

Simpler: two sub-tabs, **Deploy output** (build + runtime + live stream) and
**Requests** (access log). Keep the deploy picker only in Deploy output. Fix
the copy to "Showing the newest N deploys."

### 4.6 Environment: a form per key

Current: every key is a card with a "New value (blank = keep)" input, a Secret
checkbox, Save and Remove (`UI:1659-1680`). Adding a key is a separate form
below. Bulk editing is hidden in a `<details>` (`UI:1745-1750`). Inherited keys
are rows with no action (`UI:1712-1717`).

Simpler: one table (Key · Value · Scope · Origin) with an edit disclosure per
row, and the bulk `.env` editor as the primary "Edit all" action. Inherited rows
get an `Override` button that copies the key into this env. Add "Import
`.env`" by pasting text; it is the workflow people already have.

### 4.7 Domains + certificates: two tables, no DNS

Current: default host sentence, custom-domain table, add form, then a separate
certificate table (`UI:1814-1837`, `UI:1319-1391`). The empty state says "The
default hostname **below** already works" but the hostname is above it
(`UI:1815` vs `UI:1827-1831`).

Simpler: one **Domains & TLS** table: host, role (primary/custom/preview), DNS
target with copy, status (verified / unknown / wrong target), certificate state
and expiry, actions. Add a small "How to point DNS" disclosure. Make the
consequence of making a domain primary explicit: the default host stops
routing.

### 4.8 Proxy: an expert form with no presets

Current: raw headers textarea, basic-auth user/password, redirect select,
compression, force HTTPS, max body (`UI:1839-1899`). Six unrelated concerns in
one column, with `X-Frame-Options` examples in a placeholder.

Simpler: three groups — **Security** (force HTTPS, security-headers preset),
**Access** (basic auth on/off), **Performance** (compression, max body) — and an
"Advanced headers" disclosure for the textarea. Presets are what 80% want.

### 4.9 Settings: three products in one page

Current: Git repository, Framework, then GitHub webhook (payload URL, branch,
write-only secret), then Danger zone (`UI:1901-2006`). "Or initialize a
server-side git remote with `dok git-init hello`" is a CLI instruction in a web
form (`UI:1996-2000`).

Simpler: **Deploy source** (tabs: CLI push / Git remote / GitHub webhook, each
with the exact command or URL and a copy button), **Project** (name, team,
resources port/memory/CPU), **Danger zone**. Hide the git-init path under
"Advanced" with the command and a copy button. This also gives the currently
missing place to edit `dok.toml` values.

### 4.10 Account: a passkey card that cannot manage passkeys

Current: "Add a backup passkey so losing one device does not lock you out", one
button (`UI:2271-2274`), then token creation, token table, sign out everywhere
(`UI:2265-2278`). There is no list of registered passkeys, creation date, or
remove action.

Simpler: a passkey table (name/created/last used/remove) plus "Add passkey".
Token table gets last used and a "copy login command" action. Group under
**Security** and **CLI access**.

### 4.11 Team: role explanation after the control

Current: invite role select then a one-line paragraph explaining roles
(`UI:2471-2476`). The select lists member/viewer/admin/owner with no per-role
description.

Simpler: role cards or inline hints next to the select ("Member can deploy and
read; cannot change settings"). Keep the paragraph as fallback.

### 4.12 Sidebar: three flat lists and no state

Current: Teams, Members & invites, Projects, Account, version
(`UI:883-953`). Project entries are plain names; no status, no environment, no
search. With 20 projects it becomes a scroll.

Simpler: team switcher at top, `Overview` entry, project list with a tiny
status dot and a filter box when >8 projects. Keep `Members & invites` under a
team menu rather than a peer of projects.

## 5. What would make the dashboard feel full and useful

Five additions, in order. Each uses data dok already has unless noted.

### 5.1 Team pulse: a recent-activity stream

**Where:** dashboard, below the health strip, or as a right rail on wide
screens. **What:** the last 10 events across all projects in the team:

```
just now   hello    preview  ok      022f11ae  e2e   "Add hero section"
2m ago     blog     prod     failed  bc2dc9e   e2e   cannot detect a build  [View log]
1h ago     notes    prod     ok      80947708  e2e   "Fix typo"             [Open]
```

Data: `deploys` per app (`db.deploys(app, 5)`) sorted by `created_at`; project,
env, status, author, commit message, age, link to `/logs?deploy=id`. First
version needs no schema change. This single element changes the home page from
"four cards" to "the team's work".

**Empty state:** instead of an empty dashed box, show the setup checklist (5.5).

### 5.2 Fleet health strip + filters

**Where:** directly under the team title. **What:**

```
4 projects · 3 live · 1 failed · 1 building · 1 preview        [All] [Live] [Failed] [Previews]
```

Clicking a chip filters the grid; `Failed` is emphasized when non-zero. Add
`last deploy: 2m ago` and a "certs expiring" chip when applicable. On mobile the
strip becomes two rows of chips.

Data: `app.status`, `production_deploy`, `db.preview_hosts_for`, cert status
computation already used by `cert_table` (`UI:1319-1391`). Effort is mostly
layout and a query per app; cache or reuse the status poll payload
(`UI:2739-2759`) to avoid N+1 queries.

### 5.3 Deploy row that tells the story

**Where:** deployment history + card. **What:** a state column that changes over
time:

```
● Building   2m 14s   docker build…          [Open log] [Cancel]   (cancel = P1)
● Starting   3s       container did not answer yet
● Live       1m 02s   13565d13  "Add hero"   [Open] [Logs]
● Failed     12s      cannot detect a build: add index.html…  [Open log] [Retry]
```

For failures, show the **first actionable line** from the deploy log (the CLI
already produces these, e.g. "cannot detect a build: add a Dockerfile,
package.json…"). For success, show total duration. Use `created_at` for elapsed
time now; add `finished_at`/phase later.

**Acceptance feel:** after `dok push`, the user can keep the browser open and
see the exact phase without guessing whether the tab is stale.

### 5.4 Project card that reports production truth

**Where:** dashboard grid. **What:**

```
hello                                          ● Live
hello.apps.localhost                           prod · 13565d13 · 2m ago
e2e · "Add hero section"                       1 preview →        [Open] [Logs]
```

Rules:

- The main badge/URL/time describe the **production** deploy.
- If the newest deploy is a preview, show a separate chip `1 preview →` that
  links to the previews section, never replace the production line.
- A failed project shows `● Failed` plus the error excerpt and a `View log`
  link; a project with no deploy shows `Not deployed` and a copyable
  `dok push` command.
- Remove the orphan `· 022f11ae` line; deploy ids belong in secondary text.

### 5.5 First-run setup checklist

**Where:** dashboard, replacing the single "New here?" notice that only appears
when the user has no tokens (`UI:1237-1249`). **What:** a live checklist:

```
Get your first deploy live                                     2 of 4
✓ Create a team
✓ Create an API token
○ Install the CLI        curl -fsSL https://…/install.sh | sh   [Copy]
○ Push your project      dok login https://… <token>            [Copy]
                         dok push ./my-app --app my-app         [Copy]
```

Each step auto-checks from server state: token exists (`tokens_for_user`),
project exists, first successful deploy exists. A project with a first deploy
shows a dismissible "Setup complete — invite a teammate" card.
This also fixes the missing `/install.sh` route: serve it, or at minimum link
to a self-hosted `/docs` page instead of GitHub (`UI:974`, `UI:1246`).

### Honorable mention: Observability sub-page

Once the five above exist, add a **Requests** sub-tab with a 24h requests/5xx/p95
chart, top paths, and a request-id search that links error pages to the exact
access-log row. The data is already in `access_log` (`db.rs:1740-1777`); the
missing column is `request_id`.

## 6. Simplification backlog

### P0 — do now (trust + first value)

| # | Change | Acceptance criteria |
|---|---|---|
| P0.1 | **Production truth on project cards** | A project with a newer preview still shows its production URL, status, deploy id and age as the primary state. Preview count appears as a separate chip linking to previews. No unlabeled 8-char id on the card. `ui.rs:1135-1136`, `1183-1187`, `1213-1214`. |
| P0.2 | **Failure triage** | Any failed app shows a one-line error excerpt and a `View log` link on its card and on the project overview. Clicking opens `/logs?deploy=<id>`. No failed project shows "static files · served from disk". `ui.rs:2118-2127`, `2146-2169`. |
| P0.3 | **Honest empty/runtime state** | A project with `production_deploy == None` shows `Not deployed yet` and the `dok push` command, never "served from disk". A project with a failed last deploy shows `Deploy failed` plus the last successful deploy if one exists. |
| P0.4 | **Team pulse on the dashboard** | The home page lists the last 10 deploys across the team's projects with project, env, status, author, age and a link. Empty teams see the checklist, not an empty grid. Uses existing `deploys` data; no new table required. |
| P0.5 | **Fleet header + filters** | Header shows `N projects · X live · Y failed · Z previews`; chips filter the grid. With a failed project, the failed chip is visually prominent. Mobile keeps the information without horizontal scroll. |
| P0.6 | **First-run checklist + copyable commands** | A fresh team sees 4 steps (team, token, CLI, first push) with copy buttons; steps auto-complete from server state; after the first successful deploy it collapses to a dismissible card. The page links to a working `/install.sh` or `/docs` route, not only GitHub. `ui.rs:1237-1249`, `ui.rs:974`. |

### P1 — next (make it a product)

| # | Change | Acceptance criteria |
|---|---|---|
| P1.1 | **Previews section** | Production is one pinned row; previews are grouped under a `Previews (N)` disclosure with branch, age, open/promote/delete. A preview never appears as the project's main state. |
| P1.2 | **Deploy progress** | Running deploys show elapsed time and a phase (upload/build/start/live). Failed rows show the first error line. Success shows duration. Requires `finished_at` or phase persistence. |
| P1.3 | **Domains & TLS table** | One table per project: host, role, DNS target + copy, verification state, cert state/expiry, actions. Making a domain primary warns that the default host will stop routing. Fix "below" → "above". |
| P1.4 | **Instance/health page** | New `/dashboard/system`: version, uptime, last backup + age, disk usage, Docker availability, certificate inventory with expiry, active deploys, user/team counts. Read-only first. |
| P1.5 | **Failure notifications** | A failed production deploy (and cert expiry <14 days) can send a generic outbound webhook and/or email. Configurable per team; off by default with a visible "Add a notification" prompt. |
| P1.6 | **Request-id access log** | Store `request_id` in `access_log`, show it in the requests table, and link the branded error page's id to the filtered row. Add path search and a time-range filter. |
| P1.7 | **Project settings completeness** | Settings can rename the project, move it between teams, and edit port/memory/CPU (currently `dok.toml`/CLI flags only). Validate name collisions and confirm destructive moves. `ui.rs:1915-1918`. |
| P1.8 | **Passkey management** | Account lists passkeys (label, created, last used) with remove; refuses removing the last passkey without a warning. `ui.rs:2271-2274`. |
| P1.9 | **Project search / switcher** | Sidebar filter and/or `Ctrl/Cmd-K` palette across projects; shows live/failed dots. Degrades to the current list under 8 projects. |
| P1.10 | **Restart/stop actions** | A running app can be restarted or stopped from the overview (owner/admin only), with confirmation and status feedback. New API route + UI. |

### P2 — later (depth, not polish)

| # | Change | Acceptance criteria |
|---|---|---|
| P2.1 | **Audit log** | Every env/domain/proxy/role/settings change records actor, time, before/after; team page can filter by actor and project. |
| P2.2 | **Per-project roles** | Team roles can be scoped per project (deploy/view/admin) without forking the team model. Existing team roles keep working. |
| P2.3 | **Production approval** | Optional per-project gate: previews promote only after an owner approves. Audit entry for approval. |
| P2.4 | **Traffic analytics** | 24h/7d requests, 5xx rate, p95 and top paths per project; no third-party analytics. |
| P2.5 | **DNS-01 / wildcard certs** | Wildcard `*.<base_domain>` issuance or documented external-proxy path with UI status. Known gap in `docs/REVIEW-round-8.md`. |
| P2.6 | **Build settings UI** | Build command, output dir, Dockerfile path, install command, and monorepo root dir per project, replacing `dok.toml` guesswork. |
| P2.7 | **Deploy retention controls** | UI for build retention and access-log retention (currently hard-coded / server flags). |

## 7. Quick wins under 1 hour

Each is a small, local change with an obvious verification.

1. **Fix the runtime lie** — in the Overview, when `app.production_deploy` is
   `None`, render `Not deployed yet` instead of `static files · served from
   disk` (`ui.rs:2118-2127`). Verify with a fresh project: the card says "Not
   deployed yet" and the push command is visible.
2. **Card production truth (minimal)** — choose the production deploy for the
   card's main badge/time when one exists; label a newer preview as a separate
   `preview` chip (`ui.rs:1135-1136`, `1183-1187`). Verify by pushing a preview:
   the card still shows the production deploy id and age.
3. **Failure excerpt** — on the project overview, if the latest production
   deploy failed, print the last non-empty line of its log (truncated) and link
   to `/logs?deploy=<id>` (`ui.rs:2146-2169`, `db.deploy_log`). Verify with the
   `broken` fixture: "cannot detect a build…" appears without opening Logs.
4. **Fix Domains copy** — "The default hostname **below**" → "above"
   (`ui.rs:1815`), and add a copy button next to the default host
   (`ui.rs:1827-1831`). Verify by reading the page.
5. **Log copy** — replace "Newest N deploys are selectable here; older builds
   remain available through the API" with "Showing the newest N deploys."
   (`ui.rs:1595`). Verify the string is gone.
6. **Access log density** — render one timestamp per row (relative age with the
   absolute time in `title`), not both stacked (`ui.rs:1533-1541`). Verify 100
   rows fit without the duplicated line.
7. **Copy login command** — add a copy button for the full
   `dok login <url> <token>` line in the new-token notice (`ui.rs:2253-2262`).
   Verify the clipboard content is the command, not just the token.
8. **Header account link** — make the user email in the header link to
   `/dashboard/account` (`ui.rs:974`). Verify by clicking it.
9. **Empty sidebar CTA** — "No projects yet" becomes a link to the create form
   (`ui.rs:919-921`). Verify focus lands on `#new-project`.
10. **Webhook copy buttons** — put copy buttons next to the payload URL and the
    `dok git-init <app>` hint (`ui.rs:1943-1962`, `ui.rs:1993-2000`). Verify the
    copied values match the displayed ones.
11. **Team page role hint** — add `title`/small text next to each role option
    ("admin: projects + settings + invites", "member: deploy + read") using the
    same wording as `ui.rs:2476`; no model change.
12. **Failed app flash on dashboard** — if any project in the team is failed,
    render a dismissible banner above the grid: `1 project needs attention →
    broken` linking to the project. Uses the data already fetched for cards.

## 8. What not to build (yet)

- **Do not add a JS framework or a SPA.** The server-rendered approach is a
  strength (instant loads, no build step, accessible forms). The gaps are
  information architecture, not interactivity.
- **Do not add vanity charts** before P0. A requests chart on top of an empty
  home does not fix trust; deploy truth does.
- **Do not add per-project RBAC** before team activity/audit. Teams cannot
  manage what they cannot see.
- **Do not add a theme switcher or a redesign pass.** The dark palette is
  coherent; the emptiness is structural.
- **Do not expose secrets** to make the env page prettier. Write-only is a
  deliberate, documented decision (`README.md:209-219`); keep it.
- **Do not add a plugin system / arbitrary build hooks** until the core deploy
  story is trustworthy.

## 9. Success test for this audit

After P0, a five-project team should be able to answer these in under five
seconds without opening a project:

1. What is live, and since when? (health strip + production-truth cards)
2. What failed, and why? (failure excerpt + log link)
3. What changed recently and who did it? (team pulse)
4. What do I do next as a new user? (checklist with copyable commands)

If those four answers are on the first screen, the dashboard stops feeling
empty even when the team has one project — because it is full of *state*, not
cards.
