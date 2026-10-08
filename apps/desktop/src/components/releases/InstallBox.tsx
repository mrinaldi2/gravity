// The Install box (H-229, UX-043 §2): an iOS package's install link, sent to
// a paired iPhone or iPad, copied, or scanned from a QR code.

import { useEffect, useState } from "react";
import type { ReactElement } from "react";
import type { InstallDevice, Release } from "../../protocol/releases";
import { errText } from "../../util";
import { deviceLine, installLine, installStage } from "./install";
import InstallQr from "./InstallQr";
import { releaseTitle } from "./labels";
import type { ReleaseInstallState } from "./useReleaseInstall";

const SENT_MS = 5_000;
const COPIED_MS = 2_000;
const NO_DEVICE = "No iPhone or iPad is paired. Pair one in Settings › Devices.";
const UNPUBLISHED =
  "This package has no iPhone build to install yet. DevOps publishes it after building.";

type Sent =
  | { readonly kind: "ok"; readonly text: string }
  | { readonly kind: "failed"; readonly device: InstallDevice; readonly text: string };

function SendButton(props: {
  /** Null while the daemon hasn't answered. */
  readonly devices: readonly InstallDevice[] | null;
  readonly version: string;
  readonly disabled: boolean;
  readonly now: () => number;
  readonly onSend: (device: InstallDevice) => void;
}): ReactElement {
  const { devices } = props;
  const [open, setOpen] = useState(false);
  if (devices === null) {
    return (
      <button type="button" disabled>
        Send to phone
      </button>
    );
  }
  const [only] = devices;
  if (devices.length === 0 || only === undefined) {
    return (
      <>
        <button type="button" disabled aria-describedby="install-no-device">
          Send to phone
        </button>
        <p id="install-no-device" className="release-hint">
          {NO_DEVICE}
        </p>
      </>
    );
  }
  if (devices.length === 1) {
    return (
      <button type="button" disabled={props.disabled} onClick={() => props.onSend(only)}>
        Send to {only.name ?? "phone"}
      </button>
    );
  }
  return (
    <span className="install-send">
      <button
        type="button"
        disabled={props.disabled}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        Send to phone ▾
      </button>
      {open ? (
        <ul role="menu" className="install-devices">
          {devices.map((d) => (
            <li key={d.device_id} role="none">
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setOpen(false);
                  props.onSend(d);
                }}
              >
                {deviceLine(d, props.version, props.now())}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </span>
  );
}

/** The last send's line and how to send again; a sent line stays 5 s. */
function useSend(install: ReleaseInstallState): {
  readonly sent: Sent | null;
  readonly send: (device: InstallDevice) => void;
} {
  const [sent, setSent] = useState<Sent | null>(null);
  useEffect(() => {
    if (sent?.kind !== "ok") {
      return;
    }
    const timer = window.setTimeout(() => setSent(null), SENT_MS);
    return () => window.clearTimeout(timer);
  }, [sent]);
  const run = async (device: InstallDevice): Promise<void> => {
    const name = device.name ?? "the phone";
    try {
      const offer = await install.send(device.device_id);
      const text = offer.delivered
        ? `✓ Sent to ${name}. Tap the notification there to install.`
        : `Sent to ${name}. It shows next time The Hermes opens there, under Releases.`;
      setSent({ kind: "ok", text });
    } catch (e) {
      setSent({ kind: "failed", device, text: `Couldn't send it to ${name}: ${errText(e)}` });
    }
  };
  return { sent, send: (device) => void run(device) };
}

/** Copy link: the HTTPS install page, never the itms-services link. */
function CopyLink(props: { readonly page: string; readonly disabled: boolean }): ReactElement {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) {
      return;
    }
    const timer = window.setTimeout(() => setCopied(false), COPIED_MS);
    return () => window.clearTimeout(timer);
  }, [copied]);
  const copy = async (): Promise<void> => {
    try {
      await navigator.clipboard.writeText(props.page);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };
  return (
    <button
      type="button"
      aria-label="Copy the install link"
      disabled={props.disabled}
      onClick={() => void copy()}
    >
      {copied ? "✓ Link copied" : "Copy link"}
    </button>
  );
}

function SendStatus(props: {
  readonly sent: Sent;
  readonly onRetry: (device: InstallDevice) => void;
}): ReactElement {
  const { sent } = props;
  return (
    <p className="install-status" role="status">
      {sent.text}
      {sent.kind === "failed" ? (
        <>
          {" · "}
          <button type="button" onClick={() => props.onRetry(sent.device)}>
            Try again
          </button>
        </>
      ) : null}
    </p>
  );
}

/** Send, Copy and the QR code, disabled while the build site is off. */
function InstallActions(props: {
  readonly release: Release;
  readonly install: ReleaseInstallState;
  readonly now: () => number;
}): ReactElement {
  const { info, error } = props.install;
  const { sent, send } = useSend(props.install);
  const off = info?.site !== undefined && info.site.serving !== true;
  const ready = info !== null && !off;
  const page = info?.page_url ?? "";
  const version = info?.version ?? releaseTitle(props.release);
  const qrLabel = `QR code for the install page of ${info?.app_title ?? "The Hermes"} ${version}`;
  return (
    <>
      {error ? <p className="release-hint">Couldn&apos;t read the install link: {error}</p> : null}
      {off ? (
        <p className="release-warning" role="status">
          The build site on {info?.computer ?? "this computer"} isn&apos;t serving, so the install
          page can&apos;t open. Ask DevOps to start it.
        </p>
      ) : null}
      <div className="install-actions">
        <SendButton
          devices={info === null ? null : (info.devices ?? [])}
          version={version}
          disabled={!ready}
          now={props.now}
          onSend={send}
        />
        <CopyLink page={page} disabled={!ready} />
      </div>
      {sent ? <SendStatus sent={sent} onRetry={send} /> : null}
      {page ? (
        <div className={ready ? "install-qr" : "install-qr off"}>
          <InstallQr text={page} label={qrLabel} />
          <p className="release-hint">
            Scan with the iPhone camera, then tap Install. The phone must be on your tailnet.
          </p>
        </div>
      ) : null}
    </>
  );
}

export default function InstallBox({
  release,
  install,
  now = Date.now,
}: {
  readonly release: Release;
  readonly install: ReleaseInstallState;
  readonly now?: () => number;
}): ReactElement | null {
  const stage = installStage(release);
  if (stage === "hidden") {
    return null;
  }
  const { info } = install;
  const title =
    release.status === "awaiting_owner"
      ? "Install on iPhone or iPad to test"
      : "Install on iPhone or iPad";
  const unpublished = stage === "unpublished" || (info !== null && !info.page_url);
  return (
    <section className="install-box" aria-label={title}>
      <h3>{title}</h3>
      <p className="release-meta">{installLine(release, info)}</p>
      {unpublished ? (
        <p className="release-hint">{UNPUBLISHED}</p>
      ) : (
        <InstallActions release={release} install={install} now={now} />
      )}
    </section>
  );
}
