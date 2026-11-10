// Screenshot helper for brand verification.
// usage: NODE_PATH=/tmp/dok-e2e/node_modules node brand/verify.mjs [file|all]
import { chromium } from "playwright";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";
import fs from "node:fs";

const root = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const mode = args[0] || "all";

const targets = [
  { src: "explore/index.html", out: "screens/explore.png", w: 1280, h: 1100 },
  { src: "preview.html", out: "screens/preview-1600x1200.png", w: 1600, h: 1200, full: true },
  { src: "preview.html", out: "screens/preview-390x844.png", w: 390, h: 844, full: true },
];

fs.mkdirSync(path.join(root, "screens"), { recursive: true });

const browser = await chromium.launch();
const errors = [];
for (const t of targets) {
  if (mode !== "all" && mode !== "explore" && mode !== "preview") {
    if (!t.src.includes(mode)) continue;
  }
  if (mode === "explore" && !t.src.includes("explore")) continue;
  if (mode === "preview" && !t.src.includes("preview")) continue;
  const page = await browser.newPage({
    viewport: { width: t.w, height: t.h },
    deviceScaleFactor: 2,
  });
  page.on("console", (m) => m.type() === "error" && errors.push(`${t.src}: ${m.text()}`));
  page.on("pageerror", (e) => errors.push(`${t.src}: ${e.message}`));
  await page.goto(pathToFileURL(path.join(root, t.src)).href, { waitUntil: "load" });
  await page.waitForTimeout(400);
  const out = path.join(root, t.out);
  await page.screenshot({ path: out, fullPage: !!t.full });
  // report any horizontal overflow
  const over = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth
  );
  if (over > 1) errors.push(`${t.src} @${t.w}px: horizontal overflow ${over}px`);
  console.log(`${out}  ${t.w}x${t.h}${t.full ? " (full)" : ""}`);
  await page.close();
}
await browser.close();
if (errors.length) {
  console.error("PROBLEMS:\n" + errors.join("\n"));
  process.exitCode = 1;
} else {
  console.log("ok");
}
