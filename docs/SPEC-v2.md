# dok v2 — Vercel-lite, self-hosted

> Historical design note. The shipped behavior is documented in `README.md`;
> the API/UI lists below are a subset and may lag behind the code.

## Cíl
Open-source platforma jako Vercel, ale lehká: jeden Rust binary, SQLite, Docker.
Nasadí si ji kdokoliv u sebe; funguje pro jednotlivce i týmy.

## Model
- **User** — passkey login (Proton Pass, iCloud, Google, hardware).
- **Team** — vlastní projekty; role owner/admin/member/viewer; pozvánky odkazem.
- **Project (app)** — jeden deployovatelný web; globálně unikátní jméno = subdoména.
- **Deployment** — production / preview; histories, promote, rollback.
- **Env** — proměnné per prostředí (production / preview / development), secret flag.
- **Domain** — vlastní doména nebo `<app>.<base>`, cert řeší edge (Traefik) nebo ACME.

## Buildy
1. `Dockerfile` (volitelně `dockerfile = "..."` v `dok.toml`).
2. Statika (bez Dockerfile, žádný známý projekt).
3. Auto-detekce (bez Dockerfile): `package.json` → Node, `requirements.txt`/`pyproject.toml` → Python, `go.mod` → Go; vygeneruje Dockerfile.
4. `dok.toml`: `name`, `port`, `domain`, `dockerfile`, `memory_mb`, `cpus`, `framework`.

## API (v1)
```
POST /api/v1/deploy                      upload tarball (X-Dok-Project, X-Dok-Env)
GET  /api/v1/projects
POST /api/v1/projects                    {name}
GET  /api/v1/projects/{app}
DELETE /api/v1/projects/{app}
GET  /api/v1/projects/{app}/deployments
POST /api/v1/projects/{app}/promote/{id}
POST /api/v1/projects/{app}/rollback/{id}
GET  /api/v1/projects/{app}/logs?env=
GET  /api/v1/projects/{app}/logs/stream?env=
PUT  /api/v1/projects/{app}/env/{env}
GET  /api/v1/projects/{app}/env/{env}
POST /api/v1/projects/{app}/domains
GET  /api/v1/projects/{app}/metrics
POST /api/v1/teams/{team}/invites
```
Staré `/apps/...` zůstává jako alias.

## UI
Server-rendered, bez JS frameworku. Sidebar (tým + projekty), project page s taby:
Deployments, Logs (live), Environment, Domains, Settings. Tmavý, minimalistický.

## Kritéria kritiků (9/10)
- Feature parity s Vercel core: projekty, preview, promote, rollback, env, domény, logy, tým.
- UX: jasná hierarchie, žádné mrtvé konce, prázdné stavy, mobil.
- Engineering: authz na každém endpointu, žádné race, testy, migrace, žádná tajemství.

## Edge: proxy a certifikáty (Coolify-like)

dok musí umět být sám edge, ne jen za cizím Traefikem:

1. **Vestavěný edge režim** (`dok serve --edge`):
   - :80 (ACME HTTP-01 + redirect na HTTPS) a :443 (TLS-ALPN-01 i normální provoz).
   - Certy pro všechny domény z DB (custom domény + `<app>.<base_domain>`), obnova před expirací.
   - Úložiště certů na disku (`<data>/acme`), stav v UI.
   - Když edge neběží, chová se jako dnes (plain HTTP za externím proxy).
2. **Per-projekt proxy nastavení**:
   - vlastní hlavičky (X-Frame-Options, CSP…),
   - basic auth (user + hash),
   - redirect www ↔ apex,
   - komprese (gzip/br),
   - max body size,
   - force HTTPS.
3. **UI**: záložka Domains rozšířená o stav certu (issued / expiring / error),
   tlačítko renew, a nová záložka Proxy (hlavičky, basic auth, redirect, komprese).
4. **API**: `GET/PUT /api/v1/projects/{app}/proxy`, `GET /api/v1/certificates`,
   `POST /api/v1/certificates/{host}/renew`.
5. **Access log** posledních N requestů na projekt (čas, status, latence, IP) —
   ukládat do DB s retencí, zobrazit v UI.

Kritérium: na novém VPS s veřejnou IP stačí `dok serve --edge` a doména funguje
včetně HTTPS bez Traefiku.
