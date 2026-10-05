import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import ServiceRecoveryBanner from "./ServiceRecoveryBanner";

const OFFER = {
  title: "The last update didn't finish",
  body: "Finishing the update installs the current service.",
  action: "Finish the update",
} as const;

describe("ServiceRecoveryBanner", () => {
  it("runs the install only on its one action", async () => {
    const user = userEvent.setup();
    const onInstall = vi.fn<() => void>();
    const onDismiss = vi.fn<() => void>();
    render(
      <ServiceRecoveryBanner
        offer={OFFER}
        installing={false}
        error={null}
        onInstall={onInstall}
        onDismiss={onDismiss}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Not now" }));
    expect(onInstall).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Finish the update" }));
    expect(onInstall).toHaveBeenCalledTimes(1);
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("shows the installer's error exactly, holder list included", () => {
    const error = "daemon install failed: Error: still in use\n  pid 7 python3 (cwd /u/.gravity)";
    render(
      <ServiceRecoveryBanner
        offer={OFFER}
        installing={false}
        error={error}
        onInstall={vi.fn<() => void>()}
        onDismiss={vi.fn<() => void>()}
      />,
    );
    expect(screen.getByLabelText("Installer output").textContent).toBe(error);
  });

  it("locks both buttons while the install runs", () => {
    render(
      <ServiceRecoveryBanner
        offer={OFFER}
        installing
        error={null}
        onInstall={vi.fn<() => void>()}
        onDismiss={vi.fn<() => void>()}
      />,
    );
    expect(screen.getByRole("button", { name: "Working…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Not now" })).toBeDisabled();
  });
});
