# Handoff: dok v2 → 9/10

## Cíl
Open-source, self-hosted Vercel/Coolify alternativa (Rust, SQLite, Docker, jeden binary).
Cíl je, aby tři kritici podle `docs/RUBRIC.md` dali každý **9/10**.

## Stav
- Repo `~/projects/dok`, branch `main`, commity lokálně — **nepushovat**.
- v2 hotové: multi-user passkey auth, týmy/role/pozvánky, sessions v SQLite, projekty,
  preview deploye s veřejnou URL `<app>-<deploy8>.<base>`, promote/rollback, retence per env,
  env per prostředí, build auto-detekce (Node/Python/Go), edge režim s ACME a cert lifecycle,
  per-projekt proxy (hlavičky, basic auth, redirect, komprese), access log, docker stats.
- Security kolo hotové: host ownership, fail-closed tokeny, team-only projects, CSRF, rate limit,
  upload capy, git transport allowlist, verzované migrace.
- Poslední kritiky: feature 5/10, UX 5/10, engineering 5/10 (engineering už opravený).

## Testy
```
cargo test                     # 38 testů
bash scripts/smoke.sh
bash scripts/docker-smoke.sh
bash scripts/git-smoke.sh
bash scripts/rollback-smoke.sh
bash scripts/preview-smoke.sh
bash scripts/auth-smoke.sh
NODE_PATH=/tmp/dok-e2e/node_modules bash scripts/e2e-auth.sh
```

## Co dodělat (UX kritika, v tomto pořadí)
1. **First run**: `dok serve` bez flagů nesmí skončit 404. Buď fail-fast s jasnou chybou, nebo
   branded stránka „dashboard not configured" s konkrétním flagem. Akceptuj `host:port`
   a `localhost` RP ID (port se nesmí přidat dvakrát).
2. **Env editor**: secret hodnoty nikdy nevracej do textarea (write-only nebo `KEY=` = beze změny),
   per-row secret checkbox, mazání klíčů, žádné tiché deklasifikování.
3. **Flash zprávy** po každém POST (uloženo/chyba) místo `let _ =`; validační chyby inline s 422,
   ne 404/500.
4. **Error stránky**: branded HTML 502/504/404 s názvem appky, časem posledního dobrého deploye
   a request id. Dashboard jen na dashboard hostu; cizí hosty generic 404, žádný seznam projektů.
5. **Logs tab**: live stream přes session (ne bearer odkaz), výběr deploye, pause/copy/download,
   prázdné stavy, access log tabulka.
6. **Teams UI**: změna role, odebrání člena, seznam/revoke pozvánek, vytvoření týmu;
   join stránka řekne tým a kdo zve; neplatný invite hned, ne až po passkey.
7. **Destruktivní akce**: confirm s výčtem co zmizí, potvrzení u domén, jednotný danger pattern.
8. **Wayfinding**: aktivní projekt v sidebaru, titulky `{projekt} · {tab} · dok`, reálná metadata
   karet, badge pro new/failed, absolutní časy, promote jen u preview deployů.
9. **Mobil/a11y**: `main` první v DOM, drawer sidebar, `:focus-visible`, labely, formuláře s Enterem,
   kontrast ≥4.5:1, 44px tap targets.

## Loop
Po opravách spusť 3 nezávislé kritiky jako fresh `opencode run` (produkt/funkce, UX/design,
engineering/security) s rubrikou z `docs/RUBRIC.md`, výsledky ulož do `docs/REVIEW-round-N.md`.
Opakuj, dokud všechny tři nedají **≥9**. Maximum 4 kola; každé kolo commit.

## Až bude hotovo
- commit lokálně (nepushovat)
- pošli mail na `jachym@solta.tech`:
  ```
  printf '...shrnutí, skóre, URL...' > /tmp/dok-mail.txt
  ssh ku-bot '~/bin/mailsend jachym@solta.tech "dok: hotovo (9/10)" /tmp/dok-mail.txt'
  ```
- Nikdy nekonči blokující otázkou; pracuj průběžně a piš, co je hotové.
- Žádná tajemství do repa.
