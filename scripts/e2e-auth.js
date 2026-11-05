const { chromium } = require("playwright");

(async () => {
  const url = process.argv[2];
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
  if (!(await page.textContent(".bar h1")).includes("dok")) fail("dashboard not rendered");

  const creds = await cdp.send("WebAuthn.getCredentials", { authenticatorId });
  if (creds.credentials.length !== 1) fail("expected 1 credential, got " + creds.credentials.length);

  await page.click("text=Log out");
  await page.waitForURL(url + "/", { timeout: 10000 });
  const btn2 = await page.textContent("#go");
  if (!btn2.includes("Sign in")) fail("expected login page, got: " + btn2);

  await page.click("#go");
  await page.waitForURL("**/dashboard", { timeout: 20000 });
  if (!(await page.textContent(".bar h1")).includes("dok")) fail("dashboard after login failed");

  await browser.close();
  console.log("e2e: OK");
})().catch((e) => {
  console.error("FAIL:", e);
  process.exit(1);
});
