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

  it("heads the installer's verbatim error with what failed", () => {
    const error = "daemon install failed: Error: still in use\n  pid 7 python3 (cwd /u/.gravity)";
    const { container } = render(
      <ServiceRecoveryBanner
        offer={OFFER}
        installing={false}
        error={error}
        onInstall={vi.fn<() => void>()}
        onDismiss={vi.fn<() => void>()}
      />,
    );
    expect(screen.getByText("Couldn't finish the update.").parentElement?.textContent).toBe(
      "Couldn't finish the update. The installer said:",
    );
    const output = container.querySelector("pre");
    expect(output?.textContent).toBe(error);
    expect(output).not.toHaveAttribute("aria-label");
    expect(screen.queryByText(/still have the Hermes folder open/)).toBeNull();
  });

  it("explains a held-open home in plain words", () => {
    render(
      <ServiceRecoveryBanner
        offer={OFFER}
        installing={false}
        error={"the home is still held open; quit these and try again:\n  pid 7 node"}
        onInstall={vi.fn<() => void>()}
        onDismiss={vi.fn<() => void>()}
      />,
    );
    expect(
      screen.getByText(
        "These programs still have the Hermes folder open. Quit them, then try again.",
      ),
    ).toBeInTheDocument();
  });

  it("locks both buttons and announces the action while it runs", () => {
    const { container } = render(
      <ServiceRecoveryBanner
        offer={OFFER}
        installing
        error={null}
        onInstall={vi.fn<() => void>()}
        onDismiss={vi.fn<() => void>()}
      />,
    );
    expect(screen.getByRole("button", { name: "Finishing the update…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Not now" })).toBeDisabled();
    expect(container.querySelector('[aria-live="polite"]')?.textContent).toBe(
      "Finishing the update…",
    );
  });
});
