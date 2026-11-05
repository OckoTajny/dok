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
dok logs <app>                   build + runtime logs
dok env <app> KEY=VALUE ...      set production env (applies on next deploy)
dok env pull <app> [env]         print env as KEY=VALUE
dok domain <app> <host>          route a domain
dok open <app>                   print the public URL
dok git <app> <repo-url>         repo used by webhooks / redeploy
dok git-init <app>               make a bare repo, print the git remote
dok redeploy <app>               clone + deploy from the repo
dok deploys <app>                recent deploy ids, env and status
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

The last three builds per app are kept for rollback; older build trees and the
previous image tag are removed on the next successful deploy.

## Deploy modes

| Mode | Trigger |
|---|---|
| Tarball | `dok push` |
| Docker | a `Dockerfile` in the uploaded directory |
| Static | no `Dockerfile` (SPA fallback to `index.html`) |
| Git | `git push` to the bare repo, `dok redeploy`, or GitHub webhook |
| Preview | `X-Dok-Env: preview` or `?env=preview` on the deploy endpoint |

`git push` deploy:

```bash
dok git-init my-app            # prints ssh://<user>@<host>/<data>/repos/my-app.git
git remote add dok ssh://<user>@<host>/<data>/repos/my-app.git
git push dok main              # post-receive hook builds and deploys
```

The hook runs `dok deploy-git <app> --data <dir>`; the ssh user needs read/write
access to that directory (or use the `file://` path locally).

GitHub webhook:

```
POST https://dok.example.com/api/v1/hooks/github/<app>?secret=$DOK_HOOK_SECRET
```

Set `DOK_HOOK_SECRET` in the server environment and use the same value as the
webhook secret.

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
GET/PUT /api/v1/projects/{app}/env/{env}
GET/POST /api/v1/projects/{app}/domains
DELETE /api/v1/projects/{app}/domains/{host}
GET  /api/v1/projects/{app}/metrics
GET/PUT /api/v1/projects/{app}/proxy
GET  /api/v1/projects/{app}/access-log
GET  /api/v1/certificates
POST /api/v1/certificates/{host}/renew
POST /api/v1/teams/{team}/invites
```

`/api/v1/apps/...` remains as an alias. A token resolves to its user; every
request is checked against the project's team (owner/admin for writes, any
member for reads). Projects outside the token's teams return 404. Tokens
created before the first user exists stay full-access for compatibility.

## Proxy, edge and certificates

Per-project proxy settings (custom headers, basic auth, www/apex redirect,
static compression, max body size, force HTTPS) are available from the Proxy
tab and the API. They are applied by the built-in proxy for every request.

Default mode is plain HTTP behind an external proxy (Traefik/Caddy), exactly
as before. `dok serve --edge` binds :80 and :443, redirects HTTP to HTTPS,
answers ACME HTTP-01 challenges, and issues certificates for every custom
domain plus `<app>.<base_domain>` into `<data>/acme`. The ACME host list is
built at startup; add domains then restart the service.

## Access log

The proxy keeps the last 200 requests per project in SQLite (time, method,
path, status, latency, client IP) with automatic retention. They are shown on
the project Logs page and exposed at `/api/v1/projects/{app}/access-log`.

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
