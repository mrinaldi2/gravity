// Getting around the whole app in tests, the way the owner does (UX-024):
// Projects → a project → its Team tab → a bot.

import { act, screen, waitFor, within } from "@testing-library/react";
import { expect } from "vitest";

/** Waits for the snapshot to land on the projects home. */
export async function waitForHome(): Promise<void> {
  await waitFor(() => {
    expect(screen.getByRole("article", { name: "Acme" })).toBeInTheDocument();
  });
  await act(async () => {});
}

/** From the home, opens Acme's Team tab. */
export async function openTeam(): Promise<void> {
  const card = screen.getByRole("article", { name: "Acme" });
  await act(async () => {
    within(card).getByRole("button", { name: "Open project" }).click();
  });
  await act(async () => {
    screen.getByRole("tab", { name: "Team" }).click();
  });
}

/** A bot's card on the Team tab. */
export function botCard(name: string): HTMLElement {
  return screen.getByRole("article", { name });
}

/** From the Team tab, opens a bot's page. */
export async function openBotFromTeam(name: string): Promise<void> {
  await act(async () => {
    within(botCard(name))
      .getByRole("button", { name: `Open ${name}` })
      .click();
  });
}

/** From a bot's page, back to its project's Team tab. */
export async function backToTeam(): Promise<void> {
  await act(async () => {
    screen.getByRole("button", { name: "‹ Team" }).click();
  });
}
