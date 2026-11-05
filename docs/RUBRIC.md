# Hodnoticí rubrika (cíl 9/10)

Kritici hodnotí **jen to, co je v repu a běží** — každé tvrzení musí mít důkaz
(`file:line`, endpoint, test, screenshot). Nic spekulativního.

## Váhy

| Oblast | Váha | 9/10 znamená |
|---|---|---|
| Pokrytí funkcí | 30 | Vercel core: projekty, deploy (push i git), preview, promote, rollback, env per prostředí, domény, logy, týmy s rolemi, pozvánky, CLI parita, metriky. Navíc Coolify core: vlastní edge (ACME certy, HTTP→HTTPS), per-projekt proxy (hlavičky, basic auth, redirect, komprese), stav certů v UI, access log. |
| UX a design | 25 | Jasná hierarchie, konzistentní komponenty, prázdné stavy, čitelné chyby, funguje na mobilu, žádné mrtvé konce. Vypadá jako produkt, ne jako admin template. |
| Inženýrství | 25 | Autorizace na každém endpointu, testy (unit + e2e), migrace, žádné race condition, limity, zálohy, žádná tajemství v repu, rozumné chybové stavy. |
| Dokumentace a ops | 10 | `install.sh`, quickstart do 5 minut, self-host, upgrade, troubleshooting. |
| Produktový lesk | 10 | Názvy, onboarding, defaulty, rychlost, žádné „TODO“ v UI. |

## Škála
- **10** — ship-ready produkt, nedá se vytknout nic podstatného.
- **9** — veřejně použitelné, jen drobnosti.
- **7–8** — funkční, ale chybí kus Vercel zkušenosti nebo je co opravit.
- **5–6** — demo, ne produkt.
- **<5** — rozbité.

## Formát nálezu
`SEVERITA (blocker/high/medium/low) | důkaz (file:line / endpoint / test) | co je špatně | jak opravit`
Max 10 nálezů na kritika, seřazené. Na konci: číslo 0–10 a tři věci, které
nejvíc brání devítce.
