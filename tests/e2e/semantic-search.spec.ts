import { expect, test } from "@playwright/test";

test("semantic mode is opt-in and returns results", async ({ page }) => {
  await page.goto("/");

  const mode = page.getByRole("combobox", { name: "Search mode" });
  await expect(mode).toHaveValue("lexical");
  await mode.selectOption("semantic");

  await page
    .getByPlaceholder("Search transcripts...")
    .fill("breaking into a computer through a security bug");
  const responsePromise = page.waitForResponse(
    (response) =>
      response.url().includes("/api/search?") && response.url().includes("mode=semantic"),
  );
  await page.getByRole("button", { name: "Search" }).click();
  const response = await responsePromise;

  expect(response.ok()).toBe(true);
  await expect(page.locator("article").first()).toBeVisible();
  await expect(page).toHaveURL(/mode=semantic/);
});
