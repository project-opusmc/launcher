async page => {
  const baseUrl = "http://127.0.0.1:1421";
  const failures = [];
  const passes = [];

  const check = async (name, assertion) => {
    try {
      await assertion();
      passes.push(name);
    } catch (error) {
      failures.push(`${name}: ${error instanceof Error ? error.message : String(error)}`);
    }
  };

  const expect = (condition, message) => {
    if (!condition) throw new Error(message);
  };

  const visible = locator => locator.waitFor({ state: "visible", timeout: 2500 });

  page.setDefaultTimeout(2000);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(baseUrl, { waitUntil: "networkidle" });
  await page.evaluate(() => localStorage.clear());
  await page.reload({ waitUntil: "networkidle" });

  await check("launcher renders the real browser preview", async () => {
    await visible(page.locator(".liquid-launcher"));
    await visible(page.getByRole("heading", { name: /ready to play/i }));
    await visible(page.getByText("OpusUser", { exact: true }).first());
  });

  await check("canvas and content occupy the usable application window", async () => {
    const geometry = await page.evaluate(() => {
      const shell = document.querySelector(".liquid-launcher");
      const body = document.querySelector(".liquid-body");
      if (!(shell instanceof HTMLElement) || !(body instanceof HTMLElement)) return null;
      const bodyRect = body.getBoundingClientRect();
      return {
        shellBackground: getComputedStyle(shell).backgroundColor,
        bodyWidth: bodyRect.width,
        viewportWidth: innerWidth,
        bodyCenter: bodyRect.left + bodyRect.width / 2,
        viewportCenter: innerWidth / 2,
      };
    });
    expect(geometry !== null, "launcher shell or content body is missing");
    expect(geometry.shellBackground === "rgb(9, 9, 11)", `canvas is ${geometry.shellBackground}`);
    expect(geometry.bodyWidth >= geometry.viewportWidth - 40, `content is capped at ${geometry.bodyWidth}px`);
    expect(Math.abs(geometry.bodyCenter - geometry.viewportCenter) <= 2, "content is not centered");
  });

  await check("Home media fills its elastic workspace without covering launch controls", async () => {
    const media = page.locator(".home-media");
    const video = media.locator("video");
    await visible(media);
    await visible(video);
    const bounds = await media.boundingBox();
    const card = await page.locator(".home-play-card").boundingBox();
    const panel = await page.locator(".home-play-panel").boundingBox();
    expect(bounds !== null, "media has no rendered bounds");
    expect(card !== null && panel !== null, "Play workspace or launch controls are missing");
    expect(Math.abs(bounds.height - (card.height - 2)) <= 2, `media height is ${bounds.height}px in a ${card.height}px workspace`);
    expect(bounds.x + bounds.width <= panel.x + 1, "media overlaps the launch controls");
    expect(card.y + card.height <= 900 - 8, "workspace extends past the window");
    const videoFit = await video.evaluate(element => getComputedStyle(element).objectFit);
    expect(videoFit === "cover", `video object-fit is ${videoFit}`);
  });

  await check("surfaces avoid glass, gradients, and shadows", async () => {
    const offenders = await page.locator(".liquid-launcher *").evaluateAll(elements =>
      elements
        .filter(element => {
          if (!(element instanceof HTMLElement) || element.getClientRects().length === 0) return false;
          const style = getComputedStyle(element);
          return (
            (style.backdropFilter && style.backdropFilter !== "none") ||
            (style.webkitBackdropFilter && style.webkitBackdropFilter !== "none") ||
            (style.backgroundImage && /gradient/i.test(style.backgroundImage)) ||
            (style.boxShadow && style.boxShadow !== "none")
          );
        })
        .slice(0, 8)
        .map(element => `${element.tagName.toLowerCase()}.${element.className}`),
    );
    expect(offenders.length === 0, `effects remain on ${offenders.join(", ")}`);
  });

  await check("primary Play action uses the soft-gray 44px contract and is hittable", async () => {
    const play = page.locator(".liquid-body").getByRole("button", { name: /^play$/i });
    await visible(play);
    const result = await play.evaluate(element => {
      const style = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      const hit = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
      return {
        height: rect.height,
        background: style.backgroundColor,
        color: style.color,
        radius: style.borderRadius,
        hit: hit === element || Boolean(hit?.closest("button") === element),
      };
    });
    expect(result.height === 44, `primary height is ${result.height}px`);
    expect(result.background === "rgb(212, 212, 216)", `primary background is ${result.background}`);
    expect(result.color === "rgb(24, 24, 27)", `primary text is ${result.color}`);
    expect(result.radius === "12px", `primary radius is ${result.radius}`);
    expect(result.hit, "primary center is covered by another element");
  });

  await check("secondary launch facts remain readable on their opaque surface", async () => {
    const failures = await page.locator(".launch-facts dt").evaluateAll(elements => {
      const rgb = value => value.match(/[\d.]+/g).slice(0, 3).map(Number);
      const luminance = value => rgb(value).reduce((total, channel, index) => {
        const normalized = channel / 255;
        const linear = normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
        return total + linear * [0.2126, 0.7152, 0.0722][index];
      }, 0);
      return elements.flatMap(element => {
        let surface = element;
        while (surface && getComputedStyle(surface).backgroundColor === "rgba(0, 0, 0, 0)") surface = surface.parentElement;
        if (!surface) return [`${element.textContent}: no opaque surface`];
        const foreground = luminance(getComputedStyle(element).color);
        const background = luminance(getComputedStyle(surface).backgroundColor);
        const ratio = (Math.max(foreground, background) + 0.05) / (Math.min(foreground, background) + 0.05);
        return ratio >= 4.5 ? [] : [`${element.textContent}: ${ratio.toFixed(2)}:1`];
      });
    });
    expect(failures.length === 0, `low-contrast launch facts: ${failures.join(", ")}`);
  });

  const destinations = ["Home", "Install", "Account", "Modules", "Utilities", "Settings", "Logs"];
  await check("top navigation exposes every existing launcher flow", async () => {
    for (const destination of destinations) {
      const button = page.getByRole("button", { name: destination, exact: true });
      await visible(button);
      const height = await button.evaluate(element => element.getBoundingClientRect().height);
      expect(height >= 40, `${destination} target is only ${height}px tall`);
    }
  });

  await check("Install and Account render their existing guarded flows", async () => {
    await page.getByRole("button", { name: "Install", exact: true }).click();
    await visible(page.getByRole("heading", { name: "Installation", exact: true }));
    await visible(page.getByRole("button", { name: /verify installation/i }));
    await visible(page.getByText("Minecraft 1.8.9 verified", { exact: true }));

    await page.getByRole("button", { name: "Account", exact: true }).click();
    await visible(page.getByRole("heading", { name: "Account", exact: true }));
    await visible(page.getByRole("button", { name: "Sign in with Microsoft", exact: true }));
    await visible(page.getByLabel("Offline profile"));
    await visible(page.getByRole("button", { name: "Select OpusUser", exact: true }));
  });

  await check("Modules is a real interactive overview", async () => {
    await page.getByRole("button", { name: "Modules", exact: true }).click();
    await visible(page.getByRole("heading", { name: "Modules", exact: true }));
    await visible(page.getByText("FPS Monitor", { exact: true }).first());
    const switchControl = page.getByRole("switch", { name: /FPS Monitor/i });
    await visible(switchControl);
    const before = await switchControl.getAttribute("aria-checked");
    await switchControl.click();
    const after = await switchControl.getAttribute("aria-checked");
    expect(before !== after, "FPS preference did not change after the safe preview click");
  });

  await check("Utilities exposes placement controls backed by preview state", async () => {
    await page.getByRole("button", { name: "Utilities", exact: true }).click();
    await visible(page.getByRole("heading", { name: "Utilities", exact: true }));
    await visible(page.getByLabel("Anchor"));
    await visible(page.getByLabel("Offset"));
    await visible(page.getByLabel("Scale"));
    await visible(page.getByLabel("Opacity"));
  });

  await check("Settings and Logs remain separate operational destinations", async () => {
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await visible(page.getByRole("heading", { name: "Settings", exact: true }));
    await visible(page.getByLabel("Allocated memory"));
    await page.getByRole("button", { name: "Logs", exact: true }).click();
    await visible(page.getByRole("heading", { name: "Logs", exact: true }));
    await visible(page.getByRole("button", { name: /copy log/i }));
  });

  await check("keyboard focus is visible without layout movement", async () => {
    const home = page.locator(".liquid-nav").getByRole("button", { name: "Home", exact: true });
    await home.click();
    const before = await home.boundingBox();
    const mouseOutline = await home.evaluate(element => getComputedStyle(element).outlineStyle);
    expect(mouseOutline === "none", "a mouse click leaves the keyboard-only focus outline on the navigation");
    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("Tab");
    expect(await home.evaluate(element => element === document.activeElement), "Tab did not reach Home");
    const focus = await home.evaluate(element => {
      const style = getComputedStyle(element);
      return { outlineStyle: style.outlineStyle, outlineWidth: Number.parseFloat(style.outlineWidth) };
    });
    const after = await home.boundingBox();
    expect(focus.outlineStyle !== "none" && focus.outlineWidth >= 2, "focus ring is not visible");
    expect(before !== null && after !== null && before.x === after.x && before.y === after.y, "focus moved the control");
  });

  await check("narrow layout has no page-level horizontal overflow", async () => {
    await page.setViewportSize({ width: 820, height: 760 });
    await page.waitForTimeout(100);
    const overflow = await page.evaluate(() => ({
      viewport: innerWidth,
      documentWidth: document.documentElement.scrollWidth,
      bodyWidth: document.body.scrollWidth,
    }));
    expect(overflow.documentWidth <= overflow.viewport, `document is ${overflow.documentWidth}px in an ${overflow.viewport}px viewport`);
    expect(overflow.bodyWidth <= overflow.viewport, `body is ${overflow.bodyWidth}px in an ${overflow.viewport}px viewport`);
    await visible(page.locator(".liquid-body").getByRole("button", { name: "Play", exact: true }));
  });

  await check("reduced motion replaces the moving Home media with its poster", async () => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.reload({ waitUntil: "networkidle" });
    const media = page.locator(".home-media");
    await visible(media);
    const state = await media.evaluate(element => {
      const video = element.querySelector("video");
      return {
        background: getComputedStyle(element).backgroundImage,
        videoDisplay: video ? getComputedStyle(video).display : "missing",
        paused: video ? video.paused : true,
      };
    });
    expect(state.videoDisplay === "none", `video display is ${state.videoDisplay}`);
    expect(state.paused, "hidden reduced-motion video is still playing");
    expect(state.background.includes("opus-night-sky-poster-v1.jpg"), `poster is ${state.background}`);
  });
  await page.emulateMedia({ reducedMotion: "no-preference" });

  if (failures.length > 0) {
    throw new Error(`Launcher visual contract failed (${failures.length}/${failures.length + passes.length}):\n- ${failures.join("\n- ")}`);
  }

  return {
    result: "PASS",
    checks: passes,
    viewports: ["1440x900", "820x760 reduced-motion"],
    url: baseUrl,
  };
}
