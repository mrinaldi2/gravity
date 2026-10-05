import type { ServiceStatus } from "../setup";

/** The one button each recovery offers; every one runs `service install`. */
type RecoveryAction = "Install the Hermes service" | "Finish the update" | "Repair the service";

export interface RecoveryOffer {
  readonly title: string;
  readonly body: string;
  readonly action: RecoveryAction;
}

/**
 * What to offer for the service state the app found, or `null` when there
 * is nothing to fix. The install that follows asks before moving any data,
 * so a pending migration only has to say that it will.
 */
export function recoveryFor(status: ServiceStatus): RecoveryOffer | null {
  switch (status.state) {
    case "healthy":
      return null;
    case "not_installed":
      return {
        title: "The Hermes service isn't installed",
        body: "Nothing on this computer runs your bots in the background. Installing the service starts it now and every time you log in.",
        action: "Install the Hermes service",
      };
    case "legacy_only":
      return {
        title: "The last update didn't finish",
        body: "Only the Hermes service from before the update is registered, and it may be switched off. Finishing the update installs the current service in its place.",
        action: "Finish the update",
      };
    case "migration_pending":
      return {
        title: "The last update didn't finish",
        body: "Your Hermes data still has to move to its new home. Finishing the update moves it and restarts every bot; you see what moves before anything happens.",
        action: "Finish the update",
      };
    case "migrated_service_missing":
      return {
        title: "The last update didn't finish",
        body: "Your Hermes data moved to its new home, but the service that runs it was never installed. Finishing the update installs it; nothing moves again.",
        action: "Finish the update",
      };
    case "unmanaged": {
      const running = status.version === null ? "A Hermes daemon" : `Hermes ${status.version}`;
      return {
        title: "The Hermes service isn't running your daemon",
        body: `${running} answers on port ${String(status.port)}, but no installed service runs it, so it won't come back after a crash or a restart. Quit the daemon you started by hand, then repair the service.`,
        action: "Repair the service",
      };
    }
    case "broken":
      return {
        title: "The Hermes service isn't running",
        body: "The service is installed, but its daemon is missing or stopped. Repairing reinstalls it from this app; your data stays where it is.",
        action: "Repair the service",
      };
  }
}
