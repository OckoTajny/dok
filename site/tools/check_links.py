#!/usr/bin/env python3
"""Check a static dok site for broken internal links, assets and anchors.

Usage: python3 tools/check_links.py /tmp/opencode/dok-site
"""
import re
import sys
from collections import Counter
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit

EXTERNAL_SCHEMES = {"http", "https", "mailto", "tel", "data", "javascript", "ftp"}
ASSET_TAGS = {"script", "img", "source", "video", "audio", "iframe", "embed", "object"}


class Page(HTMLParser):
    def __init__(self, path):
        super().__init__(convert_charrefs=True)
        self.path = path
        self.ids = []
        self.links = []  # (tag, attr, value)
        self.title = ""
        self.description = ""
        self.og_image = ""
        self.h1 = 0
        self.lang = None
        self.has_skip = False
        self._in_title = False

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "html":
            self.lang = a.get("lang")
        if "id" in a:
            self.ids.append(a["id"])
        if tag == "a":
            if "href" in a:
                self.links.append(("a", "href", a["href"]))
            if a.get("class") == "skip":
                self.has_skip = True
        elif tag == "link" and "href" in a:
            self.links.append(("link", "href", a["href"]))
        elif tag in ASSET_TAGS and "src" in a:
            self.links.append((tag, "src", a["src"]))
        elif tag == "meta" and a.get("name") == "description":
            self.description = a.get("content", "")
        elif tag == "meta" and a.get("property") == "og:image":
            self.og_image = a.get("content", "")
        elif tag == "title":
            self._in_title = True
        elif tag == "h1":
            self.h1 += 1

    def handle_endtag(self, tag):
        if tag == "title":
            self._in_title = False

    def handle_data(self, data):
        if self._in_title:
            self.title += data


def is_external(value):
    if value.startswith("//"):
        return True
    return urlsplit(value).scheme.lower() in EXTERNAL_SCHEMES


def resolve(root, page, value):
    """Return (target_path, anchor) or raise ValueError with a reason."""
    parts = urlsplit(value)
    path = unquote(parts.path)
    anchor = parts.fragment
    if not path:
        return page, anchor
    target = (page.parent / path).resolve()
    if target.is_dir():
        target = target / "index.html"
    try:
        target.relative_to(root)
    except ValueError:
        raise ValueError("escapes site root")
    return target, anchor


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    pages = sorted(root.rglob("*.html"))
    if not pages:
        print("no HTML files found")
        return 1

    parsed = {}
    for path in pages:
        p = Page(path)
        p.feed(path.read_text(encoding="utf-8"))
        p.close()
        parsed[path] = p

    errors = []
    warnings = []
    internal = 0
    anchors = 0
    external = 0

    for path, page in parsed.items():
        rel = path.relative_to(root)
        if not page.lang:
            errors.append(f"{rel}: <html> has no lang")
        if not page.title.strip():
            errors.append(f"{rel}: missing <title>")
        if not page.description.strip():
            errors.append(f"{rel}: missing meta description")
        if page.h1 != 1:
            errors.append(f"{rel}: expected exactly one <h1>, found {page.h1}")
        if not page.has_skip:
            errors.append(f"{rel}: missing skip link")
        if "main" not in page.ids:
            errors.append(f"{rel}: missing id=main for skip link")
        if page.og_image:
            og_path = urlsplit(page.og_image).path
            if not (root / og_path.lstrip("/")).is_file():
                errors.append(f"{rel}: og:image {page.og_image} -> missing file")

        dupes = [i for i, n in Counter(page.ids).items() if n > 1]
        if dupes:
            errors.append(f"{rel}: duplicate id(s): {', '.join(sorted(dupes))}")
        ids = set(page.ids)

        for tag, attr, value in page.links:
            value = value.strip()
            if not value:
                errors.append(f"{rel}: empty {attr} on <{tag}>")
                continue
            if value == "#":
                errors.append(f"{rel}: placeholder href=\"#\"")
                continue
            if is_external(value):
                if tag != "a":
                    errors.append(f"{rel}: external asset request <{tag} {attr}=\"{value}\">")
                else:
                    external += 1
                    if value.startswith("http://"):
                        warnings.append(f"{rel}: outbound http link {value}")
                continue
            internal += 1
            try:
                target, anchor = resolve(root, path, value)
            except ValueError as exc:
                errors.append(f"{rel}: {attr}=\"{value}\" {exc}")
                continue
            if not target.exists():
                errors.append(f"{rel}: {attr}=\"{value}\" -> missing {target.relative_to(root)}")
                continue
            if anchor:
                anchors += 1
                target_page = parsed.get(target)
                if target_page and anchor not in set(target_page.ids):
                    errors.append(
                        f"{rel}: {attr}=\"{value}\" -> missing anchor #{anchor} in {target.relative_to(root)}"
                    )

    # CSS must not import or fetch anything.
    for css in root.rglob("*.css"):
        text = css.read_text(encoding="utf-8")
        if "url(" in text or "@import" in text:
            errors.append(f"{css.relative_to(root)}: external CSS url()/@import found")

    # JS must not fetch external resources.
    for js in root.rglob("*.js"):
        text = js.read_text(encoding="utf-8")
        if re.search(r"\bfetch\s*\(", text):
            errors.append(f"{js.relative_to(root)}: fetch() call found")

    # Sitemap entries must map to real files.
    sitemap = root / "sitemap.xml"
    if sitemap.exists():
        text = sitemap.read_text(encoding="utf-8")
        for loc in re.findall(r"<loc>([^<]+)</loc>", text):
            path = urlsplit(loc).path
            target = (root / path.lstrip("/")).resolve()
            if path.endswith("/"):
                target = target / "index.html"
            if not target.exists():
                errors.append(f"sitemap.xml: {loc} -> missing file")
    else:
        warnings.append("sitemap.xml missing")

    if not (root / "robots.txt").exists():
        warnings.append("robots.txt missing")
    if not (root / "install.sh").exists():
        errors.append("install.sh missing")

    total_ids = sum(len(p.ids) for p in parsed.values())
    print(f"pages: {len(pages)} | internal links: {internal} | anchors: {anchors} | external links: {external} | ids: {total_ids}")
    for w in warnings:
        print(f"WARN  {w}")
    for e in errors:
        print(f"ERROR {e}")
    if errors:
        print(f"FAILED: {len(errors)} error(s)")
        return 1
    print("OK: every internal href/src and fragment resolves")
    return 0


if __name__ == "__main__":
    sys.exit(main())
