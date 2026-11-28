import { devices, expect, test, type Browser, type Page } from "@playwright/test";

const cloud = () => {
  const url = process.env.HOSTABLE_CLOUD_URL;
  if (!url) throw new Error("HOSTABLE_CLOUD_URL is not set; global setup did not start the cloud host.");
  return url;
};

async function phone(browser: Browser): Promise<Page> {
  const ctx = await browser.newContext({ ...devices["Pixel 7"], baseURL: cloud(), serviceWorkers: "block" });
  return ctx.newPage();
}

const buzz = (p: Page) => p.getByRole("button", { name: "Buzz!" });
const header = (p: Page, round: number, score: number) => p.getByText(`Round ${round} · you have ${score}`, { exact: true });

test("three phones play a live buzzer game to a winner, with a reload and a remote table", async ({ browser, request }) => {
  // (a) A live game that players join by code.
  const created = await request.post(`${cloud()}/api/games`, { data: { pack: "buzzer", pace: "live" } });
  expect(created.ok(), await created.text()).toBeTruthy();
  const { id, code } = (await created.json()) as { id: string; code: string };
  expect(code).toMatch(/^[A-Z]{4}$/);

  // (b) Phone 1 joins on / by typing the code; it becomes seat 0, the leader.
  const ana = await phone(browser);
  await ana.goto("/");
  await ana.getByLabel("Room code").fill(code.toLowerCase());
  await expect(ana.getByLabel("Room code")).toHaveValue(code);
  await ana.getByLabel("Your name").fill("Ana");
  await ana.getByRole("button", { name: "Join" }).click();
  await expect(ana.getByText("Hi, Ana!")).toBeVisible();
  await expect(ana).toHaveURL(new RegExp(`/j/${code}$`));

  // Phones 2 and 3 open the room link and type a name.
  const bo = await phone(browser);
  const cy = await phone(browser);
  for (const [p, name] of [[bo, "Bo"], [cy, "Cy"]] as const) {
    await p.goto(`/j/${code}`);
    await p.getByLabel("Your name").fill(name);
    await p.getByRole("button", { name: "Join" }).click();
    await expect(p.getByText(`Hi, ${name}!`)).toBeVisible();
  }

  // (c) Start.
  const started = await request.post(`${cloud()}/api/games/${id}/start`);
  expect(started.ok(), await started.text()).toBeTruthy();

  const phones = { Ana: ana, Bo: bo, Cy: cy };
  const scores = { Ana: 0, Bo: 0, Cy: 0 };
  type Name = keyof typeof phones;

  // (d) One phone buzzes per round; the leader starts the next one.
  async function round(n: number, winner: Name) {
    for (const [name, p] of Object.entries(phones) as Array<[Name, Page]>) {
      await expect(header(p, n, scores[name])).toBeVisible();
      await expect(buzz(p)).toBeEnabled();
    }
    await buzz(phones[winner]).click();
    scores[winner] += 1;
    if (scores[winner] === 5) return;
    await expect(phones[winner].getByText("You were first!")).toBeVisible();
    for (const [name, p] of Object.entries(phones) as Array<[Name, Page]>) {
      if (name !== winner) await expect(p.getByText(`${winner} was first`)).toBeVisible();
    }
    await ana.getByRole("button", { name: "Next round" }).click();
  }

  await round(1, "Bo");
  await round(2, "Cy");
  await round(3, "Bo");

  // (f) A fourth browser watches the table from the share link.
  const watcher = await (await browser.newContext({ baseURL: cloud(), viewport: { width: 1280, height: 800 }, serviceWorkers: "block" })).newPage();
  await watcher.goto(`/g/${id}`);
  const panel = (name: string) => watcher.locator("li.player", { hasText: name });
  await expect(panel("Ana")).toContainText("0 points");
  await expect(panel("Bo")).toContainText("2 points");
  await expect(panel("Cy")).toContainText("1 points");

  // (e) Reload Bo mid-game: same seat, same score, keeps playing.
  await expect(header(bo, 4, 2)).toBeVisible();
  await bo.reload();
  await expect(header(bo, 4, 2)).toBeVisible();
  await expect(bo.getByLabel("Your name")).toHaveCount(0);

  await round(4, "Bo");
  await round(5, "Ana");
  await round(6, "Bo");
  await round(7, "Bo");

  await expect(bo.getByText("You win!")).toBeVisible();
  await expect(ana.getByText("Bo wins", { exact: true })).toBeVisible();
  await expect(cy.getByText("Bo wins", { exact: true })).toBeVisible();

  // The watcher's table catches up to the winner by polling.
  await expect(watcher.locator(".table-status")).toHaveText("Bo wins!", { timeout: 20_000 });
  await expect(panel("Bo")).toContainText("5 points");
  await expect(panel("Ana")).toContainText("1 points");
  await expect(panel("Cy")).toContainText("1 points");
});
