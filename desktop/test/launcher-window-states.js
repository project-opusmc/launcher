// Real App + rendered UI; only native IPC is isolated. These fixtures never
// read/write real accounts, launch games, or stop real processes.
async page => {
  const names = ["zvwgvx", "Zvwgvx", "player_01", "Player_02", "Opus_03", "Opus_04", "Opus_05", "Opus_06", "Opus_07"];
  const accounts = names.map((username, index) => ({ id: `window-account-${index}`, username,
    uuid: null, kind: "offline", badge: "unofficial", ready: true, selected: index === 0 }));
  const launchPath = "Packaged game launcher ready: /Applications/Opus Launcher.app/Contents/Resources/Minecraft.app";
  const setup = `
    import { mockIPC } from "/node_modules/@tauri-apps/api/mocks.js";
    const accounts = ${JSON.stringify(accounts)};
    const state = window.__windowFixture = {
      selectedId: accounts[0].id, ready: true, unavailable: false, calls: [],
      instances: accounts.slice(1).map((account, index) => ({
        sessionId: "window-session-" + index, accountId: account.id, username: account.username,
        badge: account.badge, title: "Minecraft 1.8.9 — isolated session " + index, logDirectory: "/fixture/logs",
      })),
    };
    // Native confirmation is external to layout. Accept it only in this
    // disposable browser fixture; the real Stop handler and IPC payload run.
    window.confirm = () => true;
    mockIPC((command, payload) => {
      state.calls.push({ command, payload });
      switch (command) {
        case "launcher_snapshot":
          if (state.unavailable) throw new Error("Fixture local service unavailable");
          return {
            platform: "isolated layout fixture", dataDirectory: "/fixture",
            minecraftReady: state.ready, minecraftStatus: state.ready ? "Verified" : "Missing files",
            optifineReady: state.ready, optifineStatus: state.ready ? "Verified" : "Not verified",
            javaStatus: "Java 8u74-cacert462b08 ready", accountStored: true,
            gameLaunchReady: state.ready, gameLaunchStatus: state.ready ? ${JSON.stringify(launchPath)} : "Complete installation before launching the game.",
            buildEdition: "unified", offlineProfile: null,
            accounts: accounts.map(account => ({ ...account, selected: account.id === state.selectedId })),
            selectedAccountId: state.selectedId, activeLaunches: state.instances.length,
            activeAccountIds: state.instances.map(instance => instance.accountId),
            developerTestProfile: { available: false, active: false, simulationActive: false },
          };
        case "get_settings": return { maxMemoryMib: 2048, closeLauncherOnGameStart: false };
        case "get_utility_settings": return null;
        case "save_utility_settings": return null;
        case "account_skin": return { dataUrl: null, model: "classic", isDefault: true };
        case "list_instances": return state.instances;
        case "select_account": state.selectedId = payload.accountId; return accounts.find(account => account.id === state.selectedId);
        case "launch_game": return new Promise((resolve, reject) => { state.rejectLaunch = reject; });
        case "kill_instance": state.instances = state.instances.filter(instance => instance.sessionId !== payload.sessionId); return null;
        default: throw new Error("Unexpected native operation in window test: " + command);
      }
    }, { shouldMockEvents: true });
  `;
  await page.unrouteAll({ behavior: "wait" });
  await page.route(/\/src\/main\.tsx(?:\?.*)?$/, async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: setup + await response.text() });
  });
  const passes = [];
  const failures = [];
  const expect = (condition, message) => { if (!condition) throw new Error(message); };
  const check = async (name, assertion) => {
    try { await assertion(); passes.push(name); }
    catch (error) { failures.push(`${name}: ${error.message ?? error}`); }
  };
  const home = () => page.locator(".liquid-nav").getByRole("button", { name: "Home", exact: true });
  const primary = () => page.locator(".home-play-panel > .launcher-primary");
  const refresh = () => page.getByRole("button", { name: "Refresh launcher state", exact: true }).click();
  const assertPrimaryVisible = async () => {
    const geometry = await primary().evaluate(element => {
      const bounds = element.getBoundingClientRect();
      const hit = document.elementFromPoint(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
      return { height: bounds.height, bottom: bounds.bottom, viewport: innerHeight,
        hit: hit === element || hit?.closest("button") === element };
    });
    expect(geometry.height >= 44 && geometry.bottom <= geometry.viewport && geometry.hit,
      `primary action is clipped, compressed or covered: ${JSON.stringify(geometry)}`);
  };
  page.setDefaultTimeout(3000);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 980, height: 650 });
  await page.goto("http://127.0.0.1:1421/", { waitUntil: "networkidle" });
  await primary().waitFor({ state: "visible" });
  await page.waitForFunction(() => document.querySelectorAll(".session-row").length === 8);

  await check("long packaged path does not push account/Play off the minimum-size window", async () => {
    await assertPrimaryVisible();
    expect(!await page.locator(".home-profile-status").innerText().then(text => text.includes("/Applications")),
      "Home still renders the verbose packaged path");
    await page.getByRole("button", { name: "Install", exact: true }).click();
    expect(await page.locator(".operational-row").filter({ hasText: "Launch host" }).innerText().then(text => text.includes(launchPath)),
      "full diagnostic path is lost from Installation");
    await home().click();
  });

  await check("many running sessions scroll locally and the last Stop remains actionable", async () => {
    const localScroll = await page.locator(".session-list").evaluate(element => ({
      overflow: getComputedStyle(element).overflowY, extra: element.scrollHeight - element.clientHeight,
      pageExtra: document.querySelector(".page-viewport").scrollHeight - document.querySelector(".page-viewport").clientHeight,
    }));
    expect(localScroll.overflow === "auto" && localScroll.extra > 100 && localScroll.pageExtra <= 1,
      `sessions force the whole Home to scroll: ${JSON.stringify(localScroll)}`);
    await primary().focus();
    await page.keyboard.press("Tab");
    await page.keyboard.press("Tab");
    await page.keyboard.press("Tab");
    const regionFocus = await page.locator(".session-list").evaluate(element => ({
      focused: document.activeElement === element,
      style: getComputedStyle(element).outlineStyle,
      width: Number.parseFloat(getComputedStyle(element).outlineWidth),
      offset: Number.parseFloat(getComputedStyle(element).outlineOffset),
    }));
    expect(regionFocus.focused && regionFocus.style !== "none" && regionFocus.width >= 2 && regionFocus.offset <= 0,
      `session scroller lacks a clipping-safe keyboard focus ring: ${JSON.stringify(regionFocus)}`);
    const lastStop = page.locator(".session-row").last().getByRole("button", { name: "Stop", exact: true });
    await lastStop.focus();
    await lastStop.scrollIntoViewIfNeeded();
    const target = await lastStop.evaluate(element => {
      const bounds = element.getBoundingClientRect();
      return document.elementFromPoint(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2) === element;
    });
    expect(target, "last session Stop is covered after local scrolling");
    await lastStop.click();
    await page.waitForFunction(() => document.querySelectorAll(".session-row").length === 7);
    const stopPayload = await page.evaluate(() => window.__windowFixture.calls.find(call => call.command === "kill_instance")?.payload);
    expect(stopPayload.sessionId === "window-session-7", `Stop targets the wrong session: ${JSON.stringify(stopPayload)}`);
    // Keep the real Stop notice visible for the following launch. A notice
    // and another busy/error banner can legitimately coexist in production.
    await assertPrimaryVisible();
  });

  await check("case-distinct account selection keeps the payload and Play survives notice plus busy", async () => {
    const select = page.locator(".inline-select select");
    await select.selectOption("window-account-1");
    expect(await page.locator(".account-pill span").innerText() === "Zvwgvx", "account selection changes literal IGN");
    expect(await primary().innerText() === "In game" && await primary().isDisabled(), "running account can be launched twice");
    await select.selectOption("window-account-0");
    expect(await primary().innerText() === "Play" && await primary().isEnabled(), "idle account cannot be launched");
    await primary().click();
    await page.waitForFunction(() => typeof window.__windowFixture.rejectLaunch === "function");
    const call = await page.evaluate(() => window.__windowFixture.calls.find(call => call.command === "launch_game"));
    expect(call.payload.accountId === "window-account-0" && call.payload.settings.maxMemoryMib === 2048,
      `Play dropped account or memory: ${JSON.stringify(call)}`);
    expect(await page.locator(".state-banner").count() === 2, "notice plus busy fixture is not present");
    await assertPrimaryVisible();
    expect(await primary().isDisabled(), "busy launch is not guarded");
  });

  await check("notice plus launch error leaves the retry action visible and enabled", async () => {
    await page.evaluate(() => window.__windowFixture.rejectLaunch(new Error("Fixture launch failed; retry is available.")));
    await page.getByRole("alert").waitFor({ state: "visible" });
    await page.waitForFunction(() => !document.querySelector(".home-play-panel > .launcher-primary").disabled);
    expect(await page.locator(".state-banner").count() === 2, "notice plus error fixture is not present");
    await assertPrimaryVisible();
    const dismiss = page.getByRole("button", { name: "Dismiss", exact: true });
    while (await dismiss.count()) await dismiss.first().click();
  });

  await check("setup and unavailable states retain their guarded actions in the same shell", async () => {
    await page.evaluate(() => { window.__windowFixture.ready = false; });
    await refresh();
    await page.waitForFunction(() => document.querySelector(".home-play-panel > .launcher-primary").textContent === "Review installation");
    await assertPrimaryVisible();
    await primary().click();
    await page.getByRole("heading", { name: "Installation", exact: true }).waitFor({ state: "visible" });
    await home().click();
    await page.evaluate(() => { window.__windowFixture.unavailable = true; });
    await refresh();
    await page.getByRole("alert").waitFor({ state: "visible" });
    await assertPrimaryVisible();
    expect(await primary().isDisabled(), "unavailable state permits an unsafe launch");
    await page.evaluate(() => { window.__windowFixture.unavailable = false; window.__windowFixture.ready = true; });
    await page.getByRole("button", { name: "Try again", exact: true }).click();
    await page.waitForFunction(() => !document.querySelector(".home-play-panel > .launcher-primary").disabled);
    await assertPrimaryVisible();
  });
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await page.screenshot({ path: "/Users/zvwgvx/Project/Opus/output/playwright/ranked/launcher-window-states.png" });
  await page.unrouteAll({ behavior: "wait" });
  if (failures.length) throw new Error(`Window states failed (${failures.length}/${failures.length + passes.length}):\n- ${failures.join("\n- ")}`);
  return { result: "PASS", checks: passes, nativeBoundary: "isolated IPC; real App and UI", viewport: "980x650" };
}
