import type { Story } from "@ladle/react";
import { recoveryFor } from "../../app/serviceRecovery";
import type { RecoveryOffer } from "../../app/serviceRecovery";
import type { ServiceState } from "../../setup";
import ServiceRecoveryBanner from "./ServiceRecoveryBanner";

const noop = (): void => {};

function offer(state: ServiceState, version: string | null = null): RecoveryOffer {
  const found = recoveryFor({ state, port: 49777, version });
  if (found === null) {
    throw new Error(`no offer for ${state}`);
  }
  return found;
}

export const NotInstalled: Story = () => (
  <ServiceRecoveryBanner
    offer={offer("not_installed")}
    installing={false}
    error={null}
    onInstall={noop}
    onDismiss={noop}
  />
);

export const FinishTheUpdate: Story = () => (
  <ServiceRecoveryBanner
    offer={offer("legacy_only")}
    installing={false}
    error={null}
    onInstall={noop}
    onDismiss={noop}
  />
);

export const RepairWithInstallerError: Story = () => (
  <ServiceRecoveryBanner
    offer={offer("unmanaged", "0.14.2")}
    installing={false}
    error={
      "daemon install failed: Error: the home is still held open; quit these and try again:\n" +
      "  pid 4120 hermesd (cwd /Users/owner/.thehermes)\n" +
      "  pid 4388 node (cwd /Users/owner/.thehermes/projects/gravity/bots/lead/workspace)"
    }
    onInstall={noop}
    onDismiss={noop}
  />
);

export const Installing: Story = () => (
  <ServiceRecoveryBanner
    offer={offer("broken")}
    installing
    error={null}
    onInstall={noop}
    onDismiss={noop}
  />
);
