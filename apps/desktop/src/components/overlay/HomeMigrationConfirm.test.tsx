import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { confirmHomeMigration } from "../../app/homeMigration";
import HomeMigrationConfirm from "./HomeMigrationConfirm";

const SUMMARY = "Moves /u/.gravity to /u/.thehermes, restarts 3 bot(s).";

describe("HomeMigrationConfirm", () => {
  it("shows nothing until an install asks", () => {
    const { container } = render(<HomeMigrationConfirm />);
    expect(container).toBeEmptyDOMElement();
  });

  it("shows the summary with Cancel focused, so Enter declines", async () => {
    const user = userEvent.setup();
    render(<HomeMigrationConfirm />);
    let answer: Promise<boolean> = Promise.resolve(true);
    act(() => {
      answer = confirmHomeMigration(SUMMARY);
    });

    expect(screen.getByText(/restarts 3 bot\(s\)/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
    await user.keyboard("{Enter}");

    await expect(answer).resolves.toBe(false);
    expect(screen.queryByText(/restarts 3 bot\(s\)/)).not.toBeInTheDocument();
  });

  it("confirms only on the explicit button", async () => {
    const user = userEvent.setup();
    render(<HomeMigrationConfirm />);
    let answer: Promise<boolean> = Promise.resolve(false);
    act(() => {
      answer = confirmHomeMigration(SUMMARY);
    });

    await user.click(screen.getByRole("button", { name: "Move and restart" }));

    await expect(answer).resolves.toBe(true);
  });
});
