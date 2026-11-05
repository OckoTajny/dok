# dok

Tiny self-hosted deploy platform in one Rust binary. Think "Vercel/Coolify, but small":

- `dok push` uploads a directory and deploys it
- Dockerfile → build + run container, otherwise the directory is served statically
- built-in reverse proxy with host routing (`app.example.com` or `<app>.base.domain`)
- automatic Let's Encrypt certificates (TLS-ALPN-01)
- git deploy: `git push`, GitHub webhook, or `dok redeploy`
- passkey dashboard (Proton Pass, iCloud, Google, hardware keys) — no passwords
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
dok serve --http-port 8080 --base-domain example.com
# prints an initial token on first start
```

On your laptop:

```bash
dok login http://server:8080 <token>
dok push ./my-app --app my-app --domain my-app.example.com
```

## CLI

```
dok login <url> <token>          save server + token (~/.config/dok/config.toml)
dok push [dir] [--app N] [--domain H] [--port P]
dok apps                         list apps
dok logs <app>                   build + runtime logs
dok env <app> KEY=VALUE ...      set env (applies on next deploy)
dok domain <app> <host>          route a domain
dok git <app> <repo-url>         repo used by webhooks / redeploy
dok git-init <app>               make a bare repo, print the git remote
dok redeploy <app>               clone + deploy from the repo
dok deploys <app>                recent deploy ids and status
dok rollback <app> <deploy-id>   redeploy an earlier build
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
On first visit you register a passkey; that becomes the admin login. Any
WebAuthn provider works, including Proton Pass, iCloud Keychain, Google
Password Manager and hardware keys. No passwords, no tokens in the browser.
The dashboard lists apps, their status and domains, and has **redeploy**,
**logs** and **rollback** buttons. Sessions are stored in SQLite, so a service
restart does not log you out; dashboard actions are CSRF-protected.

Sessions live in memory, so a server restart logs you out (the passkey stays).

## Roadmap

- [x] push, docker, static, proxy, ACME, webhook, CLI
- [x] passkey dashboard (Proton Pass passkeys)
- [x] `git push dok main` via bare repos + `post-receive`
- [x] zero-downtime switch (new container first), per-app memory/CPU limits
- [x] rollback, retention, daily DB backups, persistent sessions
- [ ] `dok logs -f`
- [ ] one-command install script

## License

MIT
