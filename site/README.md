# dok.djt-group.com

Static marketing and documentation site for **dok**, the self-hosted deploy
platform in one Rust binary. No build step, no framework, no external requests.

## Preview

```bash
python3 -m http.server 8099 -d /tmp/opencode/dok-site
# http://localhost:8099
```

## Deploy with dok

From this directory:

```bash
dok push . --app docs --domain dok.djt-group.com
```

Point `dok.djt-group.com` at the dok host first — a CNAME to the dashboard host,
or an A record to the server IPv4. The site is plain files; dok serves it as a
static project and issues the certificate.

## Structure

- `index.html` — marketing page: promise, terminal demo, quickstart, features,
  comparison, security, docs CTA.
- `docs/` — nine pages: overview, quickstart, CLI, deploy, domains, teams,
  self-hosting, API, troubleshooting.
- `assets/style.css` — the whole design system, light and dark.
- `assets/app.js` — theme, mobile nav, copy buttons, search palette, filters,
  TOC scrollspy.
- `assets/search-index.js` — generated search entries for every docs section.
- `install.sh` — the real installer, served from this domain.
- `tools/check_links.py` — internal link, anchor and asset checker.

## Checks

```bash
python3 tools/check_links.py /tmp/opencode/dok-site
```

The checker verifies every internal `href`/`src` and fragment resolves, rejects
external asset requests, and checks titles, descriptions and duplicate ids.

## Editing

The HTML is hand-written and static. Keep paths relative: docs pages use
`../assets/...` and `../index.html`; home uses `docs/...`. The theme preference
is stored in `localStorage` under `dok-theme`; light is the default.

`docs/index.html` has a card filter; the header search opens a palette with `/`
or <kbd>Ctrl/Cmd K</kbd>. The search index is generated from the docs headings —
regenerate it if you add a section (or edit `assets/search-index.js` by hand).

## Notes

- No external fonts, images, analytics or CDNs. All icons are inline SVG.
- Outbound links go only to GitHub and the live instance.
- "Edit on GitHub" points at the dok repository; update it if the site source
  moves to its own repo.
