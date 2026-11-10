# Handoff: dok → 9/10 (kolo 5)

## Stav
- v2 žije na `https://dok.solta.tech`; kód je na desktopu `~/projects/dok`, commity lokálně.
- Kola kritiků: 1) 7.5/7.0, 2) 7/7.5/8, 3) 7/8.3/7, **4) 8/8/8**.
- Všechny nálezy z kola 4 jsou opravené a commitnuté (`fix: close round-4 findings`).
- `cargo test`: 58 zelených.

## Úkol
1. Spusť **kolo 5** kritiků (3 čerstvé `opencode run` podle `docs/RUBRIC.md`), ulož `docs/REVIEW-round-5.md`.
2. Pokud některý dá < 9, oprav **blocker/high** a pokud možno i medium, commitni, spusť kolo 6 (max 6 kol celkem).
3. Když všichni tři dají **≥ 9/10**:
   - `cargo build --release`
   - `install -m755 target/release/dok ~/bin/dok`
   - `systemctl --user restart dok`
   - ověř `https://dok.solta.tech` = 200 a `hello.apps.solta.tech` = 200
   - pošli mail:
     ```
     printf 'dok 9/10\n\n<shrnutí>\nSkóre: produkt X, UX Y, engineering Z\nhttps://dok.solta.tech\n' > /tmp/dok-mail.txt
     ssh ku-bot '~/bin/mailsend jachym@solta.tech "dok: hotovo (9/10)" /tmp/dok-mail.txt'
     ```
4. Když se 9/10 nedaří, nesnižuj laťku: vypiš konkrétní zbývající mezery a pošli mail s aktuálním skóre
   a seznamem toho, co chybí.

## Testy
`cargo test`, `scripts/{smoke,docker-smoke,git-smoke,rollback-smoke,preview-smoke,auth-smoke}.sh`,
`NODE_PATH=/tmp/dok-e2e/node_modules scripts/e2e-auth.sh`.

## Pravidla
- Commituj lokálně, nepushuj. Neber produkční DB ani nemaž appky.
- Pracuj průběžně, nikdy nekonči blokující otázkou.
