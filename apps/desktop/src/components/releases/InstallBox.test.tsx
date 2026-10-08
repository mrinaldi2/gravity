import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactElement } from "react";
import { encode } from "uqr";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InstallDevice, Release, ReleaseInstall } from "../../protocol/releases";
import { FakeDaemon } from "../../test/fakeDaemon";
import { release } from "../../test/releaseFixtures";
import { deviceLine, installLine, installStage } from "./install";
import InstallBox from "./InstallBox";
import { useReleaseInstall } from "./useReleaseInstall";

const NOW = Date.parse("2026-10-08T12:00:00Z");
const PAGE = "https://mac.tail.ts.net/releases/rel-1/ios/index.html";
const LINK =
  "itms-services://?action=download-manifest&url=https://mac.tail.ts.net/releases/rel-1/ios/manifest.plist";
const IPHONE: InstallDevice = {
  device_id: "d1",
  name: "iPhone 16",
  connected: true,
  last_seen_at: "2026-10-08T11:59:00Z",
};
const IPAD: InstallDevice = { device_id: "d2", name: "iPad", last_seen_at: "2026-10-08T10:00:00Z" };

function ios(over: Partial<Release> = {}, published = true): Release {
  return release({
    status: "approved",
    display_version: "0.6.1",
    builds: [
      {
        platform: "ios",
        version: "12",
        artifact: "/releases/rel-1/ios/TheHermes.ipa",
        url: published ? "https://mac.tail.ts.net/releases/rel-1/ios/TheHermes.ipa" : null,
        install_url: published ? LINK : null,
        sha256: "c".repeat(64),
        built_at: "2026-10-08T09:00:00Z",
      },
    ],
    ...over,
  });
}

function info(over: Partial<ReleaseInstall> = {}): ReleaseInstall {
  return {
    release_id: "rel-1",
    version: "0.6.1",
    build: "12",
    state: "approved",
    installable: true,
    page_url: PAGE,
    install_url: LINK,
    site: { serving: true },
    computer: "mac",
    devices: [IPHONE],
    app_title: "The Hermes",
    ...over,
  };
}

function Box(props: { readonly release: Release; readonly client: FakeDaemon }): ReactElement {
  const install = useReleaseInstall(props.client, props.release);
  return <InstallBox release={props.release} install={install} now={() => NOW} />;
}

function daemon(answer: ReleaseInstall): FakeDaemon {
  return new FakeDaemon()
    .onRequest("release_install", () => ({ type: "release_install", req_id: "1", install: answer }))
    .onRequest("release_send_to_device", (req) => {
      if (req.type !== "release_send_to_device") {
        throw new Error(`unexpected ${req.type}`);
      }
      return {
        type: "install_offer",
        req_id: "2",
        offer: {
          release_id: req.release_id,
          device_id: req.device_id,
          device_name: "iPhone 16",
          install_url: LINK,
          delivered: req.device_id === "d1",
        },
      };
    });
}

describe("InstallBox", () => {
  let writeText: ReturnType<typeof vi.fn<(text: string) => Promise<void>>>;
  beforeEach(() => {
    writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
  });

  it("offers Send, Copy and a QR code of the install page on an approved package", async () => {
    const client = daemon(info());
    render(<Box release={ios()} client={client} />);
    const box = screen.getByRole("region", { name: "Install on iPhone or iPad" });
    expect(within(box).getByText("The Hermes 0.6.1 (12) · approved")).toBeInTheDocument();
    const copy = await within(box).findByRole("button", { name: "Copy the install link" });
    await waitFor(() => expect(copy).toBeEnabled());
    await userEvent.click(copy);
    expect(writeText).toHaveBeenCalledWith(PAGE);
    expect(await within(box).findByText("✓ Link copied")).toBeInTheDocument();

    const qr = within(box).getByRole("img", {
      name: "QR code for the install page of The Hermes 0.6.1",
    });
    const cells = encode(PAGE, { ecc: "M", border: 2 }).data;
    const path = cells
      .flatMap((row, y) => row.map((dark, x) => (dark ? `M${x} ${y}h1v1h-1z` : "")))
      .join("");
    expect(qr.querySelector("path")?.getAttribute("d")).toBe(path);
    expect(within(box).getByText(/^Scan with the iPhone camera/)).toBeInTheDocument();
    expect(client.requests[0]?.body).toEqual({
      type: "release_install",
      release_id: "rel-1",
      check_site: true,
    });
  });

  it("sends to the one paired phone by name and says where to tap", async () => {
    const client = daemon(info());
    render(<Box release={ios()} client={client} />);
    const send = await screen.findByRole("button", { name: "Send to iPhone 16" });
    await waitFor(() => expect(send).toBeEnabled());
    await userEvent.click(send);
    expect(client.requests.at(-1)?.body).toEqual({
      type: "release_send_to_device",
      release_id: "rel-1",
      device_id: "d1",
    });
    expect(
      await screen.findByText("✓ Sent to iPhone 16. Tap the notification there to install."),
    ).toBeInTheDocument();
  });

  it("lists several devices with when each was seen, and says when one is away", async () => {
    const client = daemon(info({ devices: [IPHONE, IPAD] }));
    render(<Box release={ios()} client={client} />);
    const send = await screen.findByRole("button", { name: "Send to phone ▾" });
    await waitFor(() => expect(send).toBeEnabled());
    await userEvent.click(send);
    expect(screen.getByRole("menuitem", { name: "iPhone 16 · connected" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("menuitem", { name: "iPad · seen 2h ago" }));
    expect(
      await screen.findByText(
        "Sent to iPad. It shows next time The Hermes opens there, under Releases.",
      ),
    ).toBeInTheDocument();
  });

  it("says why a send failed and tries again", async () => {
    const client = daemon(info());
    client.onRequest("release_send_to_device", () => {
      throw new Error("no paired device d1");
    });
    render(<Box release={ios()} client={client} />);
    const send = await screen.findByRole("button", { name: "Send to iPhone 16" });
    await waitFor(() => expect(send).toBeEnabled());
    await userEvent.click(send);
    expect(
      await screen.findByText(/Couldn't send it to iPhone 16: no paired device d1/u),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(client.requests.filter((r) => r.body.type === "release_send_to_device")).toHaveLength(2);
  });

  it("disables Send with the pairing hint when no device is paired", async () => {
    render(<Box release={ios()} client={daemon(info({ devices: [] }))} />);
    expect(
      await screen.findByText("No iPhone or iPad is paired. Pair one in Settings › Devices."),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send to phone" })).toBeDisabled();
  });

  it("says the build site isn't serving and disables every action", async () => {
    const off = info({ site: { problem: "no answer" }, computer: "Manuel's MacBook Air" });
    render(<Box release={ios()} client={daemon(off)} />);
    expect(
      await screen.findByText(
        "The build site on Manuel's MacBook Air isn't serving, so the install page can't open. Ask DevOps to start it.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send to iPhone 16" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Copy the install link" })).toBeDisabled();
    // The dimmed QR doesn't invite a scan (UX-046).
    expect(screen.queryByText(/^Scan with the iPhone camera/)).toBeNull();
  });

  it("has no buttons before the iPhone build is published", () => {
    const client = daemon(info());
    render(<Box release={ios({ status: "built" }, false)} client={client} />);
    expect(
      screen.getByText(
        "This package has no iPhone build to install yet. DevOps publishes it after building.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button")).toBeNull();
    expect(client.requests).toHaveLength(0);
  });

  it("is titled for testing while the package awaits the owner", async () => {
    const client = daemon(info({ for_testing: true, state: "awaiting_owner" }));
    render(<Box release={ios({ status: "awaiting_owner" })} client={client} />);
    expect(
      screen.getByRole("region", { name: "Install on iPhone or iPad to test" }),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("The Hermes 0.6.1 (12) · ready for you to test"),
    ).toBeInTheDocument();
  });
});

describe("install words", () => {
  it("shows the box only for an iOS package in an install state", () => {
    expect(installStage(release())).toBe("hidden");
    expect(installStage(ios())).toBe("offered");
    expect(installStage(ios({ status: "rejected" }))).toBe("hidden");
    expect(installStage(ios({ status: "held" }))).toBe("hidden");
    expect(installStage(ios({ status: "assembling" }, false))).toBe("unpublished");
    expect(installStage(ios({ status: "deploying" }, false))).toBe("unpublished");
  });

  it("names the version and build once, and a device already on it", () => {
    expect(installLine(ios({ status: "deployed" }), info({ build: "0.6.1" }))).toBe(
      "The Hermes 0.6.1 · live",
    );
    expect(deviceLine({ ...IPHONE, app_version: "0.6.1 (12)" }, "0.6.1", NOW)).toBe(
      "iPhone 16 · has 0.6.1",
    );
    expect(deviceLine({ device_id: "d3", name: "Old" }, "0.6.1", NOW)).toBe("Old · never seen");
    expect(
      deviceLine(
        { device_id: "d4", name: "Far", last_seen_at: "2026-10-05T12:00:00Z" },
        "0.6.1",
        NOW,
      ),
    ).toBe("Far · seen 3d ago");
  });
});
