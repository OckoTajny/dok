# Handoff: dok

## Cíl
Lehký self-hosted PaaS v Rustu (push, Docker, statika, proxy, passkey dashboard).
**Běží na desktopu**, ku-bot je jen veřejná TLS brána. Nic se nestaví na ku-botu.

## Stav (hotovo)
- Repo: `~/projects/dok`, branch `main`, commity lokálně.
- Služby na desktopu (user systemd): `dok.service` (port 8080), `dok-traefik-sync.path` + `.service`.
- Veřejně: `https://dok.solta.tech` (passkey zatím neregistrovaný), `https://hello.apps.solta.tech`, `https://docker-hello.apps.solta.tech`.
- Traefik na ku-botu proxyuje `dok.solta.tech` a `*.apps.solta.tech` na `http://100.88.8.51:8080` (tailscale).
- Data: `~/.local/share/dok` (dok.db, builds/, backups/, traefik/dok-apps.yaml, initial-token).
- Funkce: push, Dockerfile build/run, statika, git push deploy, GitHub webhook, redeploy, rollback (poslední 3 buildy), logy, env, domény, limity (`dok.toml memory_mb/cpus`), denní zálohy DB, persistentní sessions, CSRF.

## Testy
```
cargo test
scripts/smoke.sh scripts/docker-smoke.sh scripts/git-smoke.sh scripts/rollback-smoke.sh scripts/auth-smoke.sh
NODE_PATH=/tmp/dok-e2e/node_modules scripts/e2e-auth.sh
```

## Další kroky v pořadí
1. **Úklid ku-botu**: smazat `~/.config/systemd/user/dok.service` a `~/dok-src`; data `~/.local/share/dok` nechat jako zálohu; odebrat iptables pravidlo pro 8080; zkontrolovat, že v `/data/coolify/proxy/dynamic/` jsou jen `dok.yaml` a `dok-apps.yaml`.
2. **`install.sh`**: binárka + user unity + token na nový stroj (režim desktop / edge), idempotentní.
3. **`dok logs -f`**: live tail (dnes vrací jen posledních 200/300 řádků).
4. **Keep-awake**: desktop se nesmí uspat, jinak web padá. Přidat user unit se `systemd-inhibit` (sleep:idle) nebo vypnout idle suspend a zdokumentovat.
5. Doladit README + `docs/DEPLOY.md` pro desktop architekturu, projít CI.

## Pozor
- Traefik vydá cert jen pro přesné `Host()` pravidlo → `dok-apps.yaml` se generuje při každém deployi a syncuje na ku-bot. Nový host má cert ~10 s po prvním requestu.
- `docker.sock` na desktopu má ACL pro `jachym` (`/etc/systemd/system/docker.socket.d/dok-acl.conf`); user systemd nemá docker grupu.
- Na ku-botu jsou ACL na `/data/coolify`, `/data/coolify/proxy`, `.../dynamic` pro `ubuntu`.
- `coolify-proxy` nikdy nerestartovat (Traefik hot-reload z dynamické složky).

## Pravidla
- Po každém kroku commit lokálně, **nepushovat**.
- Nikdy nekončit blokující otázkou; pracovat průběžně a psát, co je hotové.
- Žádná tajemství do repa.
