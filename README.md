# dok

Tiny self-hosted deploy platform in one Rust binary. Think "Vercel/Coolify, but small":

- `dok push` uploads a directory and deploys it
- Dockerfile → build + run container, otherwise the directory is served statically
- built-in reverse proxy with host routing (`app.example.com` or `<app>.base.domain`)
- automatic Let's Encrypt certificates (TLS-ALPN-01)
- git deploy: `git push`, GitHub webhook, or `dok redeploy`
- passkey dashboard (Proton Pass, iCloud, Google, hardware keys) — no passwords
- teams, invite links and per-team roles (owner / admin / member / viewer)
- production and preview deployments, promote and rollback
- per-environment variables, custom domains, per-project proxy settings
- built-in edge mode (`dok serve --edge`) with ACME certificates
- SQLite, no external services

```
+---------+   push    +---------------------------------------+
|  dok    | --------> |  dok serve                            |
|  CLI    |           |  axum API + proxy + ACME + docker      |
+---------+           +---------------------------------------+
```

## Quick start

On the server:

```bash
./install.sh                       # binary + systemd user service + token
# or: DOMAIN=dok.example.com BASE_DOMAIN=apps.example.com ./install.sh
```

The installer prints the initial token. On your laptop:

```bash
dok login https://dok.example.com <token>
dok push ./my-app --app my-app
```

If the machine may sleep, install the keep-awake unit too:

```bash
sudo install -m644 deploy/dok-keepawake.service /etc/systemd/system/
sudo systemctl enable --now dok-keepawake.service
```

## CLI

```
dok login <url> <token>          save server + token (~/.config/dok/config.toml)
dok push [dir] [--app N] [--domain H] [--port P] [--env production|preview]
dok apps                         list apps (alias: dok projects)
dok projects                     list projects
dok projects create <name>       create an empty project
dok projects rm <name>           delete a project
dok logs <app> [--env E]         build + runtime logs (-f to follow)
dok env <app> KEY=VALUE ...      set production env (applies on next deploy)
dok env pull <app> [env]         print env as KEY=VALUE
dok env rm <app> KEY [env]       remove one env key
dok domain <app> <host>          route a domain
dok domains list <app>           list project domains
dok domains rm <app> <host>      remove a project domain
dok whoami                       show the user and teams behind the token
dok open <app>                   print the public URL
dok git <app> <repo-url>         repo used by webhooks / redeploy
dok git-init <app>               make a bare repo, print remote + webhook secret
dok redeploy <app>               clone + deploy from the repo
dok deploys <app>                recent deploy ids, env, commit and status
dok promote <app> <deploy-id>    run a build on production
dok rollback <app> <deploy-id>   alias for promote (roll back)
dok rm <app>                     stop container, delete app
```

`dok.toml` in the project is optional:

```toml
name = "my-app"
port = 3000
domain = "my-app.example.com"
# dockerfile = "Dockerfile"
memory_mb = 512      # docker --memory
cpus = 1.0           # docker --cpus
```

The last ten builds per environment are kept for rollback (the current
production deploy is always protected); older build trees and the previous
image tag are removed on the next successful deploy.

## Deploy modes

| Mode | Trigger |
|---|---|
| Tarball | `dok push` |
| Docker | a `Dockerfile` in the uploaded directory |
| Auto-detect | `package.json` → Node, `requirements.txt`/`pyproject.toml` → Python, `go.mod` → Go (generated Dockerfile) |
| Static | `index.html` or `framework = "static"` |
| Git | `git push` to the bare repo, `dok redeploy`, or GitHub webhook |
| Preview | `dok push --env preview`, `X-Dok-Env: preview`, or `?env=preview` |

Unknown projects without a Dockerfile, manifest or `index.html` fail with a
clear error instead of being served as static.

Preview deployments get their own public URL
`<app>-<deploy8>.<base_domain>` and their own container/static directory, so
they never touch the production pointer.

`git push` deploy:

```bash
dok git-init my-app            # prints ssh://<user>@<host>/<data>/repos/my-app.git
git remote add dok ssh://<user>@<host>/<data>/repos/my-app.git
git push dok main              # post-receive hook builds and deploys
```

The hook runs `dok deploy-git <app> --data <dir>`; the ssh user needs read/write
access to that directory (or use the `file://` path locally).

GitHub webhook (per-project secret + branch filter):

```
POST https://dok.example.com/api/v1/hooks/github/<app>
X-Hub-Signature-256: sha256=<hmac>
```

`dok git-init` generates `webhook_secret` and `branch`; `GET/PUT
/api/v1/projects/{app}/webhook` reads or rotates them. The webhook verifies the
HMAC over the raw body, ignores pushes outside the configured branch, and
records commit sha/message/author on the deploy. `DOK_HOOK_SECRET` remains a
legacy fallback for projects without a per-project secret.

## Server

```
dok serve [--data DIR] [--http-port 8080] [--base-domain example.com]
          [--public-host dok.example.com]
          [--token TOKEN]
          [--acme-domain dok.example.com ...] [--acme-email you@example.com]
          [--acme-prod]
```

- Without `--acme-domain` it speaks plain HTTP; put it behind an existing proxy
  (Traefik/Caddy) if you already have one.
- With `--acme-domain` it also listens on `:443` and issues certificates.
  Add domains **before** starting the server (or `dok domain` + restart); the
  ACME domain list is built at startup.

Everything lives in the data dir (`~/.local/share/dok` by default):
`dok.db`, `builds/`, `acme/`, `logs/`, `locks/`, `backups/`, `initial-token`.
A consistent database snapshot is written to `backups/` at startup and every
24 h (`--backup-keep`, default 7, `0` disables).

## Dashboard and passkeys

Open the server in a browser (`--dashboard-origin https://dok.example.com`).
On first visit you register a passkey and create the first user and team. Any
WebAuthn provider works, including Proton Pass, iCloud Keychain, Google
Password Manager and hardware keys. No passwords, no tokens in the browser.
Additional users join through a seven-day invite link from the team page.

Each project has tabs for **Deployments** (env badge, promote/rollback),
**Logs**, **Environment** (production / preview / development), **Domains**
(with certificate status and renew), **Proxy** and **Settings**. Sessions and
CSRF tokens live in SQLite, so a restart does not log you out; every dashboard
write is CSRF-protected.

## API (v1)

```
POST /api/v1/deploy                         upload (X-Dok-Project, X-Dok-Env)
GET  /api/v1/projects                       list projects visible to the token
POST /api/v1/projects                       {name, team?}
GET  /api/v1/projects/{app}
DELETE /api/v1/projects/{app}
GET  /api/v1/projects/{app}/deployments
POST /api/v1/projects/{app}/promote/{id}
POST /api/v1/projects/{app}/rollback/{id}
GET  /api/v1/projects/{app}/logs?env=
GET  /api/v1/projects/{app}/logs/stream?env=
GET  /api/v1/projects/{app}/env/{env}
DELETE /api/v1/projects/{app}/env/{env}/{key}
GET/POST /api/v1/projects/{app}/domains
DELETE /api/v1/projects/{app}/domains/{host}
GET  /api/v1/projects/{app}/metrics
GET/PUT /api/v1/projects/{app}/proxy
GET  /api/v1/projects/{app}/access-log
GET/PUT /api/v1/projects/{app}/webhook
GET  /api/v1/certificates
POST /api/v1/certificates/{host}/renew
GET/POST /api/v1/teams
GET  /api/v1/teams/{team}/members
PUT/DELETE /api/v1/teams/{team}/members/{user}
GET/POST /api/v1/teams/{team}/invites
DELETE /api/v1/teams/{team}/invites/{token}
GET  /api/v1/whoami
```

`/api/v1/apps/...` remains as an alias. A token resolves to its user; every
request is checked against the project's team (owner/admin for writes, any
member for reads). Projects outside the token's teams return 404. Tokens that
are not bound to a user fail closed with 401. `dok serve --token` binds the
token when exactly one user already exists; otherwise it is stored inactive
and claimed by the first dashboard setup.

## Proxy, edge and certificates

Per-project proxy settings (custom headers, basic auth with argon2 hashes,
www/apex redirect, static compression, max body size, force HTTPS) are
available from the Proxy tab and the API. They are applied by the built-in
proxy for every request. Framing/hop headers cannot be injected, and
`X-Forwarded-Proto` is only trusted from loopback/private peers or
`--trusted-proxy CIDR`.

The dashboard host and the `base_domain` apex are reserved. Projects are only
routed on custom domains or `<app>.<base_domain>`, and a project with an
explicit custom domain is not served on its default subdomain. Cookies and
Authorization headers are stripped before proxying to user containers.

Default mode is plain HTTP behind an external proxy (Traefik/Caddy), exactly
as before. `dok serve --edge` binds :80 and :443, redirects HTTP to HTTPS,
answers ACME HTTP-01 challenges, and issues certificates for every custom
domain plus `<app>.<base_domain>` into `<data>/acme`. The host set is watched
every 15 seconds and the TLS acceptor is rebuilt when a domain is added, so new
projects get certificates without a manual restart. Renewal parses the real
certificate `notAfter`, marks certificates expiring within 14 days, and a renew
request triggers a fresh ACME order without downgrading a healthy certificate.
Use `--trusted-proxy` when an external load balancer terminates TLS in front of
dok.

## Access log

The proxy keeps the last 200 requests per project in SQLite (time, method,
path, status, latency, client IP) with automatic retention. They are shown on
the project Logs page and exposed at `/api/v1/projects/{app}/access-log`.
The dashboard live stream is session-authenticated (`/dashboard/stream/<app>`),
so no API token is needed in the browser. Build logs can be selected per deploy,
and metrics show real `docker stats` CPU/memory (or explicit nulls when Docker
stats are unavailable).

## Safety limits

Uploads are capped at 200 MB streamed, 20 000 archive entries and 500 MB
uncompressed. Partial uploads and `<data>/tmp` leftovers older than 24 h are
swept at startup. Environment values are passed to Docker through a 0600
`--env-file` and deleted afterwards. The CLI never packs dotfiles (except
`.well-known`) or `.env*`, and static serving denies dotfile paths. Viewer
tokens see env key names with masked values; only owner/admin see values.

Team roles: only owners may grant admin/owner; admins may invite members and
viewers. Invites are single-use and expire after seven days.

## Roadmap

- [x] push, docker, static, proxy, ACME, webhook, CLI
- [x] passkey dashboard (Proton Pass passkeys)
- [x] `git push dok main` via bare repos + `post-receive`
- [x] zero-downtime switch (new container first), per-app memory/CPU limits
- [x] rollback, retention, daily DB backups, persistent sessions
- [x] `dok logs -f`
- [x] one-command install script
- [x] teams, invites, production/preview, promote/rollback, per-env vars
- [x] per-project proxy settings, access log, certificate status UI
- [x] built-in edge mode with HTTP-01 + TLS-ALPN-01

## License

MIT
