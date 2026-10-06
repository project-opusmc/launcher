// Exercises the real Launcher DOM. Reintroducing the 960px body cap, a fixed
// 280px Home image, or the shared route scroll position must fail this contract.
// This headless session never starts a game or changes the user's accounts.
async page => {
  const passes = [];
  const failures = [];
  const geometries = [];
  const expect = (condition, message) => { if (!condition) throw new Error(message); };
  const check = async (name, assertion) => {
    try { await assertion(); passes.push(name); }
    catch (error) { failures.push(`${name}: ${error.message ?? error}`); }
  };
  const home = () => page.locator(".liquid-nav").getByRole("button", { name: "Home", exact: true });
  const measure = () => page.evaluate(() => {
    const rect = selector => {
      const element = document.querySelector(selector);
      const bounds = element.getBoundingClientRect();
      return { x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height,
        bottom: bounds.bottom, right: bounds.right, scroll: element.scrollHeight - element.clientHeight };
    };
    const play = document.querySelector(".home-play-panel > .launcher-primary");
    const bounds = play.getBoundingClientRect();
    const hit = document.elementFromPoint(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
    return {
      viewport: { width: innerWidth, height: innerHeight },
      header: rect(".liquid-header__inner"), body: rect(".liquid-body"),
      page: rect(".page-viewport"),
      home: rect(".home-page"), card: rect(".home-play-card"), media: rect(".home-media"),
      heading: rect("#home-title"), play: rect(".home-play-panel > .launcher-primary"),
      playHittable: hit === play || hit?.closest("button") === play,
      documentOverflow: document.documentElement.scrollWidth - innerWidth,
    };
  });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("http://127.0.0.1:1421/", { waitUntil: "networkidle" });
  await page.locator(".home-play-panel > .launcher-primary").waitFor({ state: "visible" });

  for (const viewport of [{ width: 980, height: 650 }, { width: 1220, height: 760 },
    { width: 1440, height: 900 }, { width: 1920, height: 1080 }]) {
    await page.setViewportSize(viewport);
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    const geometry = await measure();
    geometries.push(geometry);
    await check(`Home fills ${viewport.width}x${viewport.height} without clipping or page scrolling`, async () => {
      expect(geometry.header.width >= viewport.width - 2, `header is capped at ${geometry.header.width}px`);
      expect(geometry.body.width >= viewport.width - 40, `body is capped at ${geometry.body.width}px`);
      expect(geometry.card.x <= 24 && viewport.width - geometry.card.right <= 24,
        `Play workspace leaves ${geometry.card.x}px / ${viewport.width - geometry.card.right}px gutters`);
      expect(geometry.heading.y >= geometry.header.bottom, "Home title is behind the navigation");
      expect(geometry.home.bottom >= viewport.height - 24 && geometry.home.bottom <= viewport.height,
        `Home ends at ${geometry.home.bottom}px instead of filling the window`);
      expect(geometry.body.scroll <= 1, `Home requires ${geometry.body.scroll}px of shared body scrolling`);
      expect(geometry.page.scroll <= 1 && geometry.home.scroll <= 1, "Home content is clipped inside its own viewport");
      expect(geometry.play.bottom <= viewport.height - 8 && geometry.play.height >= 44,
        `Play is clipped or compressed: ${JSON.stringify(geometry.play)}`);
      expect(geometry.playHittable, "Play is covered or outside the visible window");
      expect(geometry.documentOverflow <= 0, "launcher overflows horizontally");
    });
  }
  await check("Home media uses added window height, not a fixed-height article image", async () => {
    expect(geometries[3].media.height > geometries[0].media.height + 300,
      `image only grows from ${geometries[0].media.height}px to ${geometries[3].media.height}px`);
  });

  await check("Home fits a 650px native window with a title-bar allowance", async () => {
    await page.setViewportSize({ width: 980, height: 622 });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    const geometry = await measure();
    expect(geometry.page.scroll <= 1 && geometry.home.scroll <= 1, "native title bar forces Home scrolling");
    expect(geometry.playHittable && geometry.play.height >= 44, "title-bar allowance clips Play");
    expect(geometry.home.bottom <= 622, "Home extends below the native content area");
  });

  await check("navigation does not carry a long Settings scroll over the Home title", async () => {
    await page.setViewportSize({ width: 980, height: 650 });
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page.getByRole("heading", { name: "Settings", exact: true }).waitFor({ state: "visible" });
    const scrolled = await page.evaluate(() => {
      let count = 0;
      for (const element of document.querySelectorAll("main, main *")) {
        if (element instanceof HTMLElement && /auto|scroll/.test(getComputedStyle(element).overflowY)
          && element.scrollHeight > element.clientHeight + 120) {
          element.scrollTop = 120;
          if (element.scrollTop >= 120) count++;
        }
      }
      return count;
    });
    expect(scrolled > 0, "Settings fixture has no scrollable region to exercise route isolation");
    await home().click();
    await page.locator("#home-title").waitFor({ state: "visible" });
    const geometry = await measure();
    expect(geometry.heading.y >= geometry.header.bottom, `Home heading is clipped at ${geometry.heading.y}px`);
    expect(geometry.playHittable, "Play is not hittable after leaving the scrolled page");
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const title = await page.locator("#settings-title").boundingBox();
    expect(title.y >= geometry.header.bottom, "reopened Settings inherits the previous route scroll");
    await home().click();
  });
  await page.setViewportSize({ width: 1220, height: 760 });
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await page.screenshot({ path: "/Users/zvwgvx/Project/Opus/output/playwright/ranked/launcher-window-layout.png" });
  if (failures.length) throw new Error(`Window layout failed (${failures.length}/${failures.length + passes.length}):\n- ${failures.join("\n- ")}`);
  return { result: "PASS", checks: passes, geometries };
}
