# dok

Tiny self-hosted deploy platform in one Rust binary. Think "Vercel/Coolify, but small":

- `dok push` uploads a directory and deploys it
- Dockerfile → build + run container, otherwise the directory is served statically
- built-in reverse proxy with host routing (`app.example.com` or `<app>.base.domain`)
- automatic Let's Encrypt certificates (TLS-ALPN-01)
- git deploy: `git clone` on webhook/redeploy
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
dok redeploy <app>               clone + deploy from the repo
dok rm <app>                     stop container, delete app
```

`dok.toml` in the project is optional:

```toml
name = "my-app"
port = 3000
domain = "my-app.example.com"
# dockerfile = "Dockerfile"
```

## Deploy modes

| Mode | Trigger |
|---|---|
| Tarball | `dok push` |
| Docker | a `Dockerfile` in the uploaded directory |
| Static | no `Dockerfile` (SPA fallback to `index.html`) |
| Git | `dok git <app> <url>` + `dok redeploy`, or GitHub webhook |

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
`dok.db`, `builds/`, `acme/`, `initial-token`.

## Roadmap

- [x] push, docker, static, proxy, ACME, webhook, CLI
- [ ] passkey login for a small web dashboard (works with Proton Pass passkeys)
- [ ] `git push dok main` via bare repos + `post-receive`
- [ ] zero-downtime switch, per-app resource limits
- [ ] `dok logs -f`

## License

MIT
