// Real App and account UI; isolate only native IPC, never real stored identities.
// Catches WKWebView's default Cancel, missing confirmation, wrong IDs,
// premature removal, duplicate requests, and a missing error/retry path.
async page => {
  const setup = `
    import { mockIPC } from "/node_modules/@tauri-apps/api/mocks.js";
    const state = window.__removeFixture = {
      accounts: [
        { id: "lower", username: "zvwgvx", uuid: null, kind: "offline", badge: "unofficial", ready: true },
        { id: "upper", username: "Zvwgvx", uuid: null, kind: "offline", badge: "unofficial", ready: true },
      ], selectedId: "lower", calls: [], confirmCalls: 0, rejectNext: false, resolveFalse: false,
    };
    // This is the actual macOS WebView default when no confirm delegate exists.
    window.confirm = () => { state.confirmCalls++; return false; };
    mockIPC((command, payload) => {
      state.calls.push({ command, payload });
      switch (command) {
        case "launcher_snapshot": return {
          platform: "isolated removal fixture", dataDirectory: "/fixture",
          minecraftReady: true, minecraftStatus: "Verified", optifineReady: true, optifineStatus: "Verified",
          javaStatus: "Java 8 ready", accountStored: state.accounts.length > 0,
          gameLaunchReady: true, gameLaunchStatus: "Ready", buildEdition: "unified", offlineProfile: null,
          accounts: state.accounts.map(account => ({ ...account, selected: account.id === state.selectedId })),
          selectedAccountId: state.selectedId, activeLaunches: 0, activeAccountIds: [],
          developerTestProfile: { available: false, active: false, simulationActive: false },
        };
        case "get_settings": return { maxMemoryMib: 2048, closeLauncherOnGameStart: false };
        case "get_utility_settings": return null;
        case "save_utility_settings": return null;
        case "account_skin": return { dataUrl: null, model: "classic", isDefault: true };
        case "list_instances": return [];
        case "remove_account": return new Promise((resolve, reject) => {
          state.finishRemove = () => {
            if (state.rejectNext) { state.rejectNext = false; reject("Fixture removal denied. Try again."); return; }
            if (state.resolveFalse) { state.resolveFalse = false; resolve(false); return; }
            state.accounts = state.accounts.filter(account => account.id !== payload.accountId);
            if (state.selectedId === payload.accountId) state.selectedId = state.accounts[0]?.id ?? null;
            resolve(true);
          };
        });
        default: throw new Error("Unexpected removal test operation: " + command);
      }
    }, { shouldMockEvents: true });
  `;
  const expect = (condition, message) => { if (!condition) throw new Error(message); };
  const row = name => page.locator(".account-row").filter({ has: page.getByRole("button", { name: `Select ${name}`, exact: true }) });
  const calls = () => page.evaluate(() => window.__removeFixture.calls.filter(call => call.command === "remove_account"));
  await page.unrouteAll({ behavior: "wait" });
  await page.route(/\/src\/main\.tsx(?:\?.*)?$/, async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: setup + await response.text() });
  });
  page.setDefaultTimeout(3000);
  await page.setViewportSize({ width: 980, height: 650 });
  await page.goto("http://127.0.0.1:1421/", { waitUntil: "networkidle" });
  await page.locator(".liquid-nav").getByRole("button", { name: /^Account/ }).click();
  const lowerRemove = row("zvwgvx").getByRole("button", { name: "Remove", exact: true });
  await lowerRemove.click();
  const dialog = page.getByRole("alertdialog");
  expect(await dialog.count() === 1 && await dialog.isVisible(),
    "Remove must show an in-Launcher confirmation even when native window.confirm returns Cancel");
  expect(await dialog.locator(".account-username").innerText() === "zvwgvx", "confirmation changes literal IGN or targets another account");
  expect((await calls()).length === 0 && await row("zvwgvx").count() === 1,
    "opening confirmation must not call native removal or hide the profile");
  const cancel = dialog.getByRole("button", { name: "Cancel", exact: true });
  expect(await cancel.evaluate(element => document.activeElement === element), "Cancel must receive safe initial focus");
  await page.keyboard.press("Shift+Tab");
  expect(await dialog.evaluate(element => element.contains(document.activeElement)), "keyboard focus escaped confirmation");
  await page.screenshot({ path: "/Users/zvwgvx/Project/Opus/output/playwright/ranked/launcher-remove-confirmation.png" });
  await cancel.click();
  await dialog.waitFor({ state: "hidden" });
  expect(await lowerRemove.evaluate(element => document.activeElement === element), "Cancel must restore the opening Remove focus");
  await lowerRemove.click();
  await page.keyboard.press("Escape");
  await dialog.waitFor({ state: "hidden" });
  expect((await calls()).length === 0 && await page.locator(".account-row").count() === 2,
    "Cancel/Escape must leave the account catalog untouched");

  await page.evaluate(() => { window.__removeFixture.rejectNext = true; });
  await lowerRemove.click();
  const confirm = dialog.getByRole("button", { name: "Remove account", exact: true });
  await confirm.click();
  await page.waitForFunction(() => typeof window.__removeFixture.finishRemove === "function");
  // A second activation while the native request is pending must be ignored.
  await dialog.getByRole("button", { name: /Removing/ }).evaluate(element => element.click());
  expect((await calls()).length === 1 && await row("zvwgvx").count() === 1, "removal is duplicated or optimistic before native acknowledgement");
  expect(await cancel.isDisabled(), "pending destructive request can be dismissed without an outcome");
  await page.evaluate(() => window.__removeFixture.finishRemove());
  await dialog.getByRole("alert").waitFor();
  expect(await row("zvwgvx").count() === 1 && await confirm.isEnabled(), "failed removal loses the profile or cannot be retried");
  await page.evaluate(() => { window.__removeFixture.resolveFalse = true; });
  await confirm.click();
  await page.waitForFunction(() => window.__removeFixture.calls.filter(call => call.command === "remove_account").length === 2);
  await page.evaluate(() => window.__removeFixture.finishRemove());
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  expect(await dialog.count() === 1 && await dialog.isVisible() && await row("zvwgvx").count() === 1,
    "native false must not close confirmation, hide the profile or announce removal success");
  expect(await dialog.getByRole("alert").innerText().then(text => text.includes("no longer exists")) && await confirm.isEnabled(),
    "native false needs an honest error with retry/cancel");
  await confirm.click();
  await page.waitForFunction(() => window.__removeFixture.calls.filter(call => call.command === "remove_account").length === 3);
  await page.evaluate(() => window.__removeFixture.finishRemove());
  await dialog.waitFor({ state: "hidden" });
  await row("zvwgvx").waitFor({ state: "detached" });
  expect(await row("Zvwgvx").count() === 1, "success removes the case-distinct other account");
  expect(await page.locator(".account-pill span").innerText() === "Zvwgvx", "success fails to refresh authoritative selected identity");
  expect(await row("Zvwgvx").getByRole("button", { name: "Select Zvwgvx", exact: true }).evaluate(element => document.activeElement === element),
    "successful removal must focus the next profile when the opener is gone");
  expect((await calls()).every(call => call.payload.accountId === "lower"), "Remove sends an incorrect account ID");
  expect(await page.evaluate(() => window.__removeFixture.confirmCalls) === 0, "Remove still depends on unsupported native confirmation");
  await row("Zvwgvx").getByRole("button", { name: "Remove", exact: true }).click();
  await confirm.click();
  await page.waitForFunction(() => window.__removeFixture.calls.filter(call => call.command === "remove_account").length === 4);
  await page.evaluate(() => window.__removeFixture.finishRemove());
  await dialog.waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.querySelectorAll(".account-row").length === 0);
  expect((await calls())[3].payload.accountId === "upper", "last profile removal uses a stale ID");
  expect(await page.locator("#account-title").evaluate(element => document.activeElement === element),
    "last profile removal must focus the Account heading, not lose keyboard context");
  await page.unrouteAll({ behavior: "wait" });
  return { result: "PASS", checks: ["real confirmation with native Cancel", "safe focus and keyboard trap", "Cancel/Escape preserve catalog", "one-flight acknowledgement", "rejection and native false/retry", "literal case-distinct IDs", "next/last profile return focus"], boundary: "real App/UI; isolated native catalog only" };
}
