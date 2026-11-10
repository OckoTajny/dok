// Screenshot + sanity script for the dok brand system.
// usage: NODE_PATH=/tmp/dok-e2e/node_modules node brand/verify.mjs [explore|preview|all]
import { chromium } from "playwright";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";
import fs from "node:fs";

const root = path.dirname(fileURLToPath(import.meta.url));
const mode = process.argv[2] || "all";
fs.mkdirSync(path.join(root, "screens"), { recursive: true });

const targets = [
  { src: "preview.html", out: "screens/preview-1600x1200.png", w: 1600, h: 1200, scale: 1 },
  { src: "preview.html", out: "screens/preview-1600x1200-full.png", w: 1600, h: 1200, full: true },
  { src: "preview.html", out: "screens/preview-390x844.png", w: 390, h: 844, scale: 2 },
  { src: "preview.html", out: "screens/preview-390x844-full.png", w: 390, h: 844, full: true, scale: 2 },
  { src: "explore/index.html", out: "screens/explore.png", w: 1280, h: 1100, full: true },
].filter((t) => mode === "all" || t.src.includes(mode));

const browser = await chromium.launch();
const problems = [];
for (const t of targets) {
  const page = await browser.newPage({
    viewport: { width: t.w, height: t.h },
    deviceScaleFactor: t.scale ?? 1,
  });
  page.on("console", (m) => m.type() === "error" && problems.push(`${t.src}: ${m.text()}`));
  page.on("pageerror", (e) => problems.push(`${t.src}: ${e.message}`));
  page.on("requestfailed", (r) => problems.push(`${t.src}: missing ${r.url()}`));
  await page.goto(pathToFileURL(path.join(root, t.src)).href, { waitUntil: "load" });
  await page.waitForTimeout(450);

  // broken or empty images
  const broken = await page.evaluate(() =>
    [...document.images]
      .filter((i) => !i.naturalWidth || (i.complete === false))
      .map((i) => i.getAttribute("src"))
  );
  broken.forEach((b) => problems.push(`${t.src}: image did not render: ${b}`));

  const over = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth
  );
  if (over > 1) problems.push(`${t.src} @${t.w}px: horizontal overflow ${over}px`);

  const out = path.join(root, t.out);
  await page.screenshot({ path: out, fullPage: !!t.full });
  console.log(`${path.relative(root, out).padEnd(38)} ${t.w}x${t.h}${t.full ? " full" : ""}`);
  await page.close();
}
await browser.close();

if (problems.length) {
  console.error("PROBLEMS:\n" + [...new Set(problems)].join("\n"));
  process.exitCode = 1;
} else {
  console.log("ok");
}
