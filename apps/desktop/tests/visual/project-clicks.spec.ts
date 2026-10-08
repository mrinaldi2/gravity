import { expect, test } from "@playwright/test";

// H-189: on the Team tab each bot card's "Open" hit area covered the whole
// window, so every click (a tab, another bot) opened the last card's bot.
// Only a real browser lays the page out, so this runs with the VR suite.
// Playwright refuses to click an element another one covers, which is the
// failure this guards against.
test("each tab and each bot card takes its own click (H-189)", async ({ page }) => {
  await page.goto("/?story=project-window--team-clicks&mode=preview");
  const opened = page.getByTestId("opened");

  await page.getByRole("button", { name: "Open Desktop Dev" }).click();
  await expect(opened).toHaveText("Opened: b2");
  await page.getByRole("button", { name: "Open Team Lead" }).click();
  await expect(opened).toHaveText("Opened: b1");
  // Anywhere on a card opens that card's bot, not the last one's. Forced:
  // the card's own button covers its heading by design, and the point is
  // where the click lands.
  await page.getByRole("article", { name: "Desktop Dev" }).locator("h3").click({ force: true });
  await expect(opened).toHaveText("Opened: b2");

  // One tab after another on purpose: each click must land on the page the
  // previous one left, as the owner's clicks do.
  for (const name of ["Overview", "Board", "Releases", "Meetings", "Team"]) {
    const tab = page.getByRole("tab", { name, exact: true });
    // oxlint-disable-next-line no-await-in-loop
    await tab.click();
    // oxlint-disable-next-line no-await-in-loop
    await expect(tab).toHaveAttribute("aria-selected", "true");
  }
  await expect(opened).toHaveText("Opened: b2");
});
