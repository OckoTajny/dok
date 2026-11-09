# Handoff: dok v2 → 9/10 (kolo 2)

## Stav
- v2 **je nasazené** na `https://dok.solta.tech` (dok 2.0.0, desktop, systemd `dok.service`).
- `cargo test`: 48 zelených; `hello.apps.solta.tech` i `docker-hello.apps.solta.tech` 200.
- Kolo 1 kritiků: produkt **7.5/10**, engineering **7/10** (UX kritik doběhl v kole 2).
  Report: `docs/REVIEW-round-1.md`.
- Od té doby: branded error stránky, a11y, preview URL, cert lifecycle, teams API, build auto-detekce,
  spousta UX polish commitů.

## Úkol
Dohnat to na **9/10 u všech tří kritiků** a poslat mail.

1. Spusť 3 čerstvé kritiky jako `opencode run` (produkt/funkce, UX/design, engineering/security)
   podle `docs/RUBRIC.md`. Každý ať dostane jen repo + rubriku, žádný kontext z tohoto sezení.
   Časový limit ~20 min na kritika, výsledky ulož do `docs/REVIEW-round-2.md`.
2. Oprav všechny **blocker/high** nálezy. Pak znovu kritici (kolo 3), max 4 kola.
3. Po každém kole commit lokálně. **Nepushovat.**
4. **Nerestartuj živou službu** `dok.service` a nesahej do produkční DB — uživatel si může
   zrovna zakládat účet. Build/test jen lokálně.

## Až budou všichni >= 9/10
Pošli mail na `jachym@solta.tech`:
```
printf 'dok 9/10\n\n<shrnutí>\nSkóre: produkt X, UX Y, engineering Z\nhttps://dok.solta.tech\n' > /tmp/dok-mail.txt
ssh ku-bot '~/bin/mailsend jachym@solta.tech "dok: hotovo (9/10)" /tmp/dok-mail.txt'
```

## Testy
```
cargo test
bash scripts/smoke.sh
bash scripts/docker-smoke.sh
bash scripts/git-smoke.sh
bash scripts/rollback-smoke.sh
bash scripts/preview-smoke.sh
bash scripts/auth-smoke.sh
NODE_PATH=/tmp/dok-e2e/node_modules bash scripts/e2e-auth.sh
```

## Pravidla
- Pracuj průběžně, nikdy nekonči blokující otázkou.
- Žádná tajemství do repa.
- Když kritik dá < 9, vypiš konkrétní mezery a oprav je; nesnižuj laťku.
