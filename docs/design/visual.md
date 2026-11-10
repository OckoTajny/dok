# Visual redesign — “Obsidian precision”

The dashboard kept every route, form and behavior; only the presentation layer in
`src/ui.rs` changed. The goal was to turn a sparse admin page into a dense,
deliberate developer tool: near-black graphite surfaces, warm amber signal,
hairline structure and editorial type, with oversized numerals for the numbers
that matter.

## Direction

- **Dark only.** Deep graphite (`#08080a`) with a barely visible grain overlay and
  two soft radial washes (amber top-left, neutral top-right) for depth. No purple
  gradients, no glassmorphism beyond the sticky topbar.
- **Amber is the only accent.** `#ffb454` marks live state, active navigation,
  section ticks, focus rings and primary actions. Blue is used only for the
  neutral `production`/environment badge so environments stay scannable.
- **Hairlines instead of shadows.** Structure comes from 1px lines
  (`--line`, `--line-soft`, `--line-strong`); elevation is reserved for the stat
  strip, the project hero and hover lift.
- **Type carries hierarchy.** System UI stack for prose, monospace for metadata,
  URLs, IDs, eyebrows, table heads and counts. No external fonts or CDNs.
- **Numbers are oversized.** 34px tabular numerals in the dashboard stat strip,
  30px in project KPI cells, with mono labels above and small qualifiers below.

## Tokens

Defined once at the top of `CSS` and used everywhere:

| Token | Value | Use |
|---|---|---|
| `--bg` | `#08080a` | page |
| `--surface` | `#0d0d11` | cards, panels, table heads |
| `--surface-2` | `#101015` | inputs |
| `--raised` / `--raised-2` | `#15151b` / `#1d1d25` | buttons, avatars |
| `--line` | `#23232b` | default hairline |
| `--line-soft` | `#191920` | row dividers |
| `--line-strong` | `#34343f` | inputs, button borders |
| `--ink` / `--ink-2` | `#f4f3ef` / `#d8d7d1` | headings / body |
| `--muted` | `#95959f` | secondary copy |
| `--faint` | `#7f7f8a` | eyebrows, timestamps, table heads (still 5:1 on bg) |
| `--accent` / `--accent-hi` | `#ffb454` / `#ffc678` | signal, primary buttons |
| `--danger` / `--success` | `#ff7264` / `#7fe0a0` | destructive / healthy |
| `--mono` | system monospace | metadata, IDs, URLs, numerals |

Radii: 9px controls, 14px cards and panels, 18px project hero. Layout is
1280px wide with a 248px sidebar, 44px gutter and a 300px activity rail.

## What changed

- **Shell.** Sticky blurred topbar with the amber mark, docs link, and a compact
  avatar/name/team block. Sidebar groups Workspace / Members & invites / Projects
  (with count and per-project status dot) / Account, and ends in a compact user
  card with the dok version. The active item gets an amber rail and warm wash.
- **Dashboard.** Editorial page head, four-cell stat strip (projects, live,
  retained deploys, requests/hour), a two-column project grid with status dot,
  env/status pills, monospace host, last deploy metadata and a footer action row
  (Overview / Logs / Deploys). The right rail holds a recent-activity timeline,
  team summary and the CLI deploy command. The empty state is a designed
  “Get started” panel with numbered CLI steps rather than a dashed hole.
- **Project page.** Bordered hero with team/project eyebrow, oversized name,
  mono domain, status/prod/kind pills and actions; tabs with an amber underline;
  five KPI cells; denser tables; log sections with a hairline toolbar. The same
  treatment carries through Domains, Environment, Proxy and Settings, including
  a dedicated danger zone.
- **Surfaces.** Buttons, inputs, selects (custom caret), checkboxes, labels,
  details/summary, flash banners, badges, certificate tables, invite links,
  login/setup/join, and 403/404/405/413/500/502 branded error pages all use the
  same tokens. Error pages get an oversized code line and mono request metadata.
- **Motion.** One subtle `rise` entrance on main content, cards and stat cells;
  a slow pulse on running dots. Everything is disabled under
  `prefers-reduced-motion: reduce`.
- **Mobile.** One-column layout, sticky topbar, horizontally scrollable tabs,
  two-column stat/KPI grids, full-width create form (input + button in one row),
  44px minimum targets, and a drawer sidebar with scrim and Escape handling.

## Accessibility

- Body and secondary text clear WCAG AA on the page surface; `--faint` was
  raised to `#7f7f8a` (~5:1) because it labels 10px mono metadata.
- Every interactive element has an amber `:focus-visible` outline; inputs also
  get a 3px translucent ring and accent border on focus.
- Status is never color-only: dots are `aria-hidden` decoration and always sit
  next to a text badge (`OK`, `failed`, `prod`, environment name).
- Live-updating badges keep `aria-live=polite`; flash banners keep
  `role=status` / `role=alert`; the skip link and drawer keep their original
  keyboard behavior.
- Tables remain real tables with header cells; the deploy picker, status filter
  and all form labels/IDs used by JS are unchanged.
- Reduced motion, `color-scheme: dark`, mobile touch targets and the existing
  `sr`-only labels are preserved.

## Verification

```bash
export CARGO_TARGET_DIR=$HOME/.cache/dok-design-target
cargo build && cargo test          # 73 passed
NODE_PATH=/tmp/dok-e2e/node_modules bash scripts/ux-smoke.sh   # existing UX e2e
NODE_PATH=/tmp/dok-e2e/node_modules bash scripts/design-smoke.sh
```

`scripts/design-smoke.sh` starts a throwaway server, registers a passkey, pushes
three sample projects plus a preview, creates an empty “Studio” team and writes
the screenshots below at 1440×900 and 390×844.

Screenshots in `docs/design/visual/`: `dashboard-populated.png`,
`dashboard-empty.png`, `project-overview.png`, `domains.png`, `login.png`,
`mobile.png` (plus logs, environment, team, account, mobile-project and the
invalid-invite page).

## Known limits

- Fonts are whatever the OS provides (per the no-CDN constraint); macOS/Windows
  render tighter than the Linux screenshots.
- The dashboard does one deploy query (up to 200 rows) and one access-log
  aggregate per project to fill the stat strip and activity rail. Fine for a
  self-hosted instance with tens of projects; it is not a paginated analytics
  query.
- Light mode was intentionally skipped; dark is the product.
