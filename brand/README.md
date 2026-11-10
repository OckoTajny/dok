# dok — brand

The brand for **dok**, a small self-hosted deploy platform: push a folder, get a URL.

Everything here is hand-authored SVG on a 32-unit grid. No raster art, no external
fonts, no filters, no gradients. The wordmark is drawn as vector geometry, so it renders
identically everywhere and needs no font licence.

---

## 1. The idea

**The tile is the deployment. The d is carved into it.**

dok's dashboard is a grid of tiles — one per app. The mark is that same object: a rounded
square (a deployed unit) with the lowercase `d` taken out of it. The `d` is a knock-out,
not a fill, so the mark sits on any surface, dark or light, without a second version.

The ascender of the carved `d` ends in a **45° cut** — a 3.2-unit chamfer on its
top-right corner. That cut is the system's signature, and it is the only flourish in
the identity:

- the `k` in the wordmark ends on the same 45° chisel cut;
- the `k`'s lower leg lands on the baseline with the same cut;
- the mark's tile uses the same geometry family (26% corner radius, straight edges).

Nothing else is decorated. Calm surfaces, one cut.

### Directions explored (kept in `explore/`)

| Direction | Concept | Verdict |
| --- | --- | --- |
| A — planes (`explore/a-planes.svg`) | sheared deploy planes stacking into a release | renders as the generic "layers" glyph and turns to mush at 16px |
| B — step d (`explore/b1-tip-accent.svg`, `b2-core.svg`, `b3-mono.svg`) | lowercase `d` as the mark, chiselled ascender | beautiful at 180px, but in the lockup it reads "d dok" and the chisel collapses below 24px |
| C — gate (`explore/c-gate.svg`) | threshold frame, lintel, app square inside, open side out | reads as a UI corner bracket with a dot; frame disappears at 16px |
| **chosen** | **tile with the `d` carved out, chiselled ascender** | keeps B's chisel, fixes B's lockup problem: the tile reads as an icon, not as a repeated letter |

`explore/index.html` is the exploration sheet (open it in a browser).

---

## 2. Geometry (reproducible)

Mark, 32-unit box:

| Element | Value |
| --- | --- |
| tile | `2 → 30` (28 units), corner radius `7.4` (26.4%) |
| carved bowl | centre `(16.1, 19.1)`, outer Ø `15`, stroke `3.6` |
| carved counter | Ø `7.8`, D-shaped: flat on the right at `x = 19` |
| carved stem | `x 19 → 23.6` (4.6 wide), baseline `y 26.6`, top `y 4.9` |
| chamfer | 45°, 3.2 units on the ascender's top-right corner |
| optical margins | left `6.6`, right `6.4`, top `2.9`, bottom `3.4` inside the tile |
| build | one `<path>`, three contours, `fill-rule="evenodd"`, no masks |

Wordmark:

| Element | Value |
| --- | --- |
| x-height | `18` |
| ascender | `25.4` (baseline `y 28`, ascender `y 2.6`) |
| mono weight | `4.2` |
| spacing | `d→o 3.9`, `o→k 3.2` (optical: straight-to-round is wider than round-to-straight) |
| ink box | `x 1.2 → 56.4`, `y 2.6 → 28` (aspect 55.2 : 25.4) |
| `k` terminals | 45° chisel, landing exactly on x-height and baseline |

Lockup: mark at ascender height (25.4), `7.2` units of clear space, then the wordmark.
Ink box `87.8 × 25.4`.

---

## 3. Colour

The primary accent is a **warm signal amber**. It is deliberately not a startup-orange:
it is desaturated enough to sit on a near-black surface all day, and warm enough to be the
only thing that moves in the UI (live dots, deploy buttons, focus rings, the mark).

### Signal

| Token | Hex | Use |
| --- | --- | --- |
| `--signal-500` | `#E9A23B` | primary: mark tile, primary button, live state |
| `--signal-400` | `#F0B25E` | hover on dark surfaces |
| `--signal-600` | `#C9821F` | pressed, 1px inner border on instance |
| `--signal-soft` | `#FBEEDC` | 12% tint for light surfaces (badges, chips) |
| `--signal-ink` | `#2A2113` | 12% tint for dark surfaces (badge fill behind amber text) |

On amber, text is **`#0A0A0A`**, never white (contrast 9.1:1 vs 2.2:1).
Amber text is used only on `#0A0A0A` / `#000` (9.1:1) — never on white.

### Neutrals — dark (the product's home surface, matches `src/ui.rs`)

| Token | Hex | Use |
| --- | --- | --- |
| `--ink-900` | `#000000` | page canvas |
| `--ink-850` | `#050505` | cards |
| `--ink-800` | `#0A0A0A` | inputs, code blocks |
| `--ink-750` | `#111111` | hover fills |
| `--ink-700` | `#171717` | selected rows |
| `--line` | `#222222` | hairlines, card borders |
| `--line-strong` | `#333333` | inputs, buttons |
| `--text-dim` | `#BBBBBB` | secondary text |
| `--text-muted` | `#888888` | meta, timestamps |
| `--text` | `#EDEDED` | body text |
| `--text-inverse` | `#FFFFFF` | hover on text |

### Neutrals — light (docs, marketing, OG)

| Token | Hex | Use |
| --- | --- | --- |
| `--paper` | `#FAFAF9` | light canvas |
| `--paper-2` | `#F4F3F0` | light raised surface |
| `--paper-line` | `#E3E1DC` | light hairline |
| `--paper-muted` | `#6F6E68` | light secondary text |
| `--ink` | `#111213` | light body text, ink wordmark |

### Status (dashboard badges — unify with the accent)

| Token | Hex | Notes |
| --- | --- | --- |
| ok | `#7EE787` | unchanged |
| running | `#E9A23B` | **was `#E3B341`** — this is the signal colour, use the token |
| failed | `#FF7B72` | unchanged |
| preview / info | `#79C0FF` | unchanged |
| new | `#D2A8FF` | unchanged |

---

## 4. Files

| File | What it is |
| --- | --- |
| `logo-mark.svg` | the symbol, 32-unit box, transparent knock-out |
| `logo-wordmark.svg` | lowercase `dok`, paths only, `currentColor` |
| `logo-lockup.svg` | mark + wordmark, ink wordmark (light surfaces) |
| `logo-inverse.svg` | mark + wordmark, paper wordmark (dark surfaces, photos) |
| `logo-mono.svg` | one ink, one colour, knock-out `d` |
| `favicon.svg` | 32×32 build: full-bleed tile, heavier `d`, flat ascender |
| `app-icon.svg` | 512×512 app icon, amber field, ink `d`, radius 112 |
| `og.svg` | 1200×630 social card |
| `preview.html` | the system on dark and light surfaces |
| `screens/` | Playwright screenshots of `preview.html` |
| `explore/` | three directions + refinements, kept for the record |
| `verify.mjs` | screenshot/verification script |

All logo files except `favicon.svg` and `app-icon.svg` are 32-unit artwork and scale
linearly. `logo-wordmark.svg`, `logo-lockup.svg`, `logo-mono.svg` inherit
`currentColor`; override `color` on the `<svg>` (or a parent) to switch ink.

---

## 5. Clear space and minimum sizes

- **Clear space**: `1/4` of the mark's height on every side (8 of the 32 units). In the
  lockup the space between mark and wordmark is `7.2` units — keep it, do not tighten.
- **Mark**: 16px minimum. Below 24px the chamfer stops reading — use `favicon.svg`.
- **Wordmark**: 56px wide minimum (x-height ≈ 18px).
- **Lockup**: 88px wide minimum. Below that, drop the wordmark and use the mark alone.
- **Never** place the lockup on a background within 15% luminance of `#E9A23B` without
  switching to `logo-mono.svg`.

---

## 6. Usage

### Browser / dashboard

```html
<link rel="icon" href="/brand/favicon.svg" type="image/svg+xml">
<link rel="apple-touch-icon" href="/brand/app-icon.svg">
<meta property="og:image" content="https://dok.djt-group.com/og.png"> <!-- rasterise og.svg at 1200x630 -->
<meta name="theme-color" content="#0A0A0A">
```

### Topbar (dark UI): the lockup at 20px high

```html
<a class="brand" href="/dashboard" aria-label="dok">
  <img src="/brand/logo-inverse.svg" alt="dok" height="20" width="55">
</a>
```

`<img>` cannot inherit `currentColor`, so the inverse lockup is the drop-in for the dark
UI. When you need to inherit colour, inline instead — but note that **inline SVGs need
unique mask ids**: append the instance name (`dokMarkCut-topbar`).

```html
<a class="brand" href="/dashboard">
  <img src="/brand/logo-mark.svg" width="20" height="20" alt="">
  <svg viewBox="0 0 55.2 25.4" height="15" fill="none" stroke="currentColor"
       stroke-width="4.2" aria-label="dok" role="img">
    <circle cx="8.1" cy="19" r="6.9"/><circle cx="32.1" cy="19" r="6.9"/>
    <path d="M46.1 20.285 54.9 11.485" stroke-linecap="butt"/>
    <path d="M46.1 17.715 54.9 26.515" stroke-linecap="butt"/>
    <g fill="currentColor" stroke="none">
      <rect x="15" y="2.6" width="4.2" height="25.4"/>
      <rect x="44.3" y="2.6" width="4.2" height="25.4"/>
    </g>
  </svg>
</a>
```

### Login / empty states

The mark at 40px above the `Sign in` heading, 24px below it. `logo-lockup.svg` at 32px
in the corner is the alternative for the passkey screen.

### Somewhere the mark is the subject

Use the tile on the amber field version (`app-icon.svg`) for PWA / touch icons, or the
knock-out mark at any size on any surface. Never add a shadow, a glow, or a rotation.

---

## 7. Do / don't

**Do**

- keep the 45° cut as the only flourish;
- let the amber be the single warm element on a screen;
- scale the mark and wordmark as one unit in the lockup;
- use `logo-mono.svg` for faxes, invoices, single-colour print, and engraving;
- use the knock-out: the mark needs no background plate.

**Don't**

- don't re-set the wordmark in a font, and don't track it tighter than 3.2/3.9 units;
- don't fill the carved `d` with a third colour — it is always a knock-out;
- don't put the mark inside another circle, squircle or badge;
- don't rotate, skew, or stretch it — the tile must stay a square;
- don't add gradients, bevels, drop shadows, or "glass" effects;
- don't put the amber tile on amber, orange or yellow backgrounds;
- don't use the wordmark below 56px wide;
- don't write the URL inside the mark.

---

## 8. Verify

```bash
NODE_PATH=/tmp/dok-e2e/node_modules node brand/verify.mjs
```

Writes `screens/preview-1600x1200.png` and `screens/preview-390x844.png`, and fails on
console errors or horizontal overflow in `preview.html`.
