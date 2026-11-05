const { chromium } = require("playwright");

(async () => {
  const [url, statePath] = process.argv.slice(2);
  const browser = await chromium.launch();
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  const cdp = await ctx.newCDPSession(page);

  await cdp.send("WebAuthn.enable");
  const { authenticatorId } = await cdp.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });

  const fail = (m) => {
    console.error("FAIL:", m);
    process.exit(1);
  };

  await page.goto(url, { waitUntil: "domcontentloaded" });
  const btn = await page.textContent("#go");
  if (!btn.includes("Create a passkey")) fail("expected setup page, got: " + btn);

  await page.click("#go");
  await page.waitForURL("**/dashboard", { timeout: 20000 });
  const text = await page.textContent("body");
  if (!text.includes("hello")) fail("app list missing deployed app");
  if (!text.includes("rollback")) fail("deploy history missing");

  const creds = await cdp.send("WebAuthn.getCredentials", { authenticatorId });
  if (creds.credentials.length !== 1) fail("expected 1 credential");

  await ctx.storageState({ path: statePath });
  await browser.close();
  console.log("register: OK");
})().catch((e) => {
  console.error("FAIL:", e);
  process.exit(1);
});
