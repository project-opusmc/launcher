// Run with Playwright CLI run-code --filename while the Launcher dev server is up.
// Only the native IPC boundary is mocked. The production App and account UI run
// unchanged; no real accounts, credentials, clipboard, or game processes are used.
async page => {
  const names = ["zvwgvx", "Zvwgvx", "zVwGvX", "ZVWGVX", "player_01"];
  const accounts = names.map((username, index) => ({
    id: `username-fixture-${index}`, username, uuid: null,
    kind: "offline", badge: "unofficial", ready: true, selected: index === 0,
  }));
  const setup = `
    import { mockIPC } from "/node_modules/@tauri-apps/api/mocks.js";
    const usernameAccounts = ${JSON.stringify(accounts)};
    let usernameSelectedId = usernameAccounts[0].id;
    window.__usernameContractCalls = [];
    mockIPC((command, payload) => {
      window.__usernameContractCalls.push({ command, payload });
      switch (command) {
        case "launcher_snapshot": return {
          platform: "isolated browser fixture", dataDirectory: "/fixture",
          minecraftReady: true, minecraftStatus: "Verified",
          optifineReady: true, optifineStatus: "Verified", javaStatus: "Java 8",
          accountStored: true, gameLaunchReady: true, gameLaunchStatus: "Ready",
          buildEdition: "unified", offlineProfile: null,
          accounts: usernameAccounts.map(account => ({ ...account, selected: account.id === usernameSelectedId })),
          selectedAccountId: usernameSelectedId, activeLaunches: 0, activeAccountIds: [],
          developerTestProfile: { available: false, active: false, simulationActive: false },
        };
        case "get_settings": return { maxMemoryMib: 2048, closeLauncherOnGameStart: false };
        case "get_utility_settings": return null;
        case "save_utility_settings": return null;
        case "account_skin": return { dataUrl: null, model: "classic", isDefault: true };
        case "list_instances": return usernameAccounts.map(account => ({
          sessionId: account.id, accountId: account.id, username: account.username,
          badge: "unofficial", title: "Fixture session", logDirectory: "/fixture",
        }));
        case "select_account":
          usernameSelectedId = payload.accountId;
          return usernameAccounts.find(account => account.id === usernameSelectedId);
        case "save_offline_profile": return null;
        default: throw new Error("Unexpected native operation in username test: " + command);
      }
    }, { shouldMockEvents: true });
  `;
  await page.route(/\/src\/main\.tsx(?:\?.*)?$/, async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: setup + await response.text() });
  });
  const expect = (condition, message) => { if (!condition) throw new Error(message); };
  const passes = [];
  const failures = [];
  const check = async (name, assertion) => {
    try { await assertion(); passes.push(name); }
    catch (error) { failures.push(`${name}: ${error.message ?? error}`); }
  };
  const visible = locator => locator.waitFor({ state: "visible", timeout: 4000 });
  const exactText = async (locator, expected) => {
    const raw = await locator.textContent();
    const rendered = await locator.innerText();
    expect(raw === expected && rendered === expected, `expected ${expected}, raw=${raw}, rendered=${rendered}`);
  };

  page.setDefaultTimeout(3000);
  await page.setViewportSize({ width: 1280, height: 860 });
  await page.goto("http://127.0.0.1:1421/", { waitUntil: "networkidle" });
  expect(await page.evaluate(() => Array.isArray(window.__usernameContractCalls)), "isolated IPC fixture was not installed");
  await visible(page.locator(".account-pill"));
  await page.getByRole("button", { name: "Account", exact: true }).click();
  const input = page.locator("#offline-username");
  await visible(input);

  await check("IGN input explicitly disables writing assistance", async () => {
    const hints = await input.evaluate(element => ({
      capitalize: element.getAttribute("autocapitalize"),
      correct: element.getAttribute("autocorrect"),
      spelling: element.getAttribute("spellcheck"),
    }));
    expect(["none", "off"].includes(hints.capitalize), `autocapitalize is ${hints.capitalize}`);
    expect(hints.correct === "off", `autocorrect is ${hints.correct}`);
    expect(hints.spelling === "false", `spellcheck is ${hints.spelling}`);
  });

  await check("typed names and submitted native payload keep exact case", async () => {
    for (const name of names) {
      await input.fill("");
      await input.pressSequentially(name);
      expect(await input.inputValue() === name, `typing changed ${name}`);
      await page.getByRole("button", { name: "Add offline profile", exact: true }).click();
      await page.waitForFunction(expected =>
        window.__usernameContractCalls.filter(call => call.command === "save_offline_profile").at(-1)?.payload.username === expected,
        name, { timeout: 3000 },
      );
      await page.waitForFunction(() => !document.querySelector("#offline-username")?.disabled, null, { timeout: 3000 });
    }
    const submitted = await page.evaluate(() =>
      window.__usernameContractCalls.filter(call => call.command === "save_offline_profile").map(call => call.payload.username),
    );
    expect(JSON.stringify(submitted) === JSON.stringify(names), `submitted names: ${JSON.stringify(submitted)}`);
  });

  const inheritedCasing = await page.addStyleTag({ content: `
    .liquid-header__meta, .launch-facts, .account-summary, .inline-select,
    .account-row__select, .session-row, .form-stack { text-transform: capitalize; }
  ` });
  await check("saved account rows and name input ignore inherited title casing", async () => {
    for (const name of names) {
      await exactText(page.getByRole("button", { name: `Select ${name}`, exact: true }).locator("strong"), name);
    }
    expect(await input.evaluate(element => getComputedStyle(element).textTransform) === "none", "input inherits text-transform");
  });

  await check("header, Home account selector, and active sessions preserve case", async () => {
    for (const account of accounts) {
      await page.getByRole("button", { name: `Select ${account.username}`, exact: true }).click();
      await exactText(page.locator(".account-pill span"), account.username);
      await page.locator(".liquid-nav").getByRole("button", { name: "Home", exact: true }).click();
      await exactText(page.locator(".launch-facts dd").first(), account.username);
      const select = page.locator(".inline-select select");
      expect(await select.inputValue() === account.id, "case variant selected a different identity");
      for (const row of accounts) await exactText(select.locator(`option[value="${row.id}"]`), row.username);
      const sessions = await page.locator(".session-row strong").allInnerTexts();
      expect(JSON.stringify(sessions) === JSON.stringify(names), `session names: ${JSON.stringify(sessions)}`);
      await page.getByRole("button", { name: "Account", exact: true }).click();
    }
  });

  // Leave an illustrative, isolated fixture view; this is not the user's catalog.
  await inheritedCasing.evaluate(element => element.remove());
  await page.locator(".liquid-nav").getByRole("button", { name: "Account", exact: true }).click();
  await page.getByRole("button", { name: `Select ${names[0]}`, exact: true }).click();
  await page.waitForFunction(expected => document.querySelector(".account-pill span")?.textContent === expected, names[0]);
  const dismiss = page.getByRole("button", { name: "Dismiss", exact: true });
  if (await dismiss.count()) await dismiss.click();
  await input.fill(names[0]);
  await page.waitForFunction(() => Array.from(document.querySelectorAll("button")).some(button =>
    button.textContent === "Add offline profile" && !button.disabled,
  ));
  await page.screenshot({ path: "/Users/zvwgvx/Project/Opus/output/playwright/ranked/launcher-username.png" });
  if (failures.length) throw new Error(`Username contract failed (${failures.length}/${failures.length + passes.length}):\n- ${failures.join("\n- ")}`);
  return { result: "PASS", names, checks: passes, nativeBoundary: "mocked; real App and UI" };
}
