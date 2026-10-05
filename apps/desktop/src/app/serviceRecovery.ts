import type { ServiceStatus } from "../setup";

/** The one button each recovery offers; every one runs `service install`. */
type RecoveryAction = "Install the Hermes service" | "Finish the update" | "Repair the service";

export interface RecoveryOffer {
  readonly title: string;
  readonly body: string;
  readonly action: RecoveryAction;
}

/** How each action reads while it runs, and when it fails (glossary rule 6). */
const WORDING: Record<RecoveryAction, { readonly progress: string; readonly failure: string }> = {
  "Install the Hermes service": {
    progress: "Installing…",
    failure: "Couldn't install the Hermes service.",
  },
  "Finish the update": {
    progress: "Finishing the update…",
    failure: "Couldn't finish the update.",
  },
  "Repair the service": { progress: "Repairing…", failure: "Couldn't repair the service." },
};

export function progressLabel(action: RecoveryAction): string {
  return WORDING[action].progress;
}

export function failureHeadline(action: RecoveryAction): string {
  return WORDING[action].failure;
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
        body: "Your bots are still set up to run on the Hermes service from before the update, and it may be stopped. Finishing the update installs the new service in its place. Your data stays where it is.",
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
      const running = status.version === null ? "Hermes" : `Hermes ${status.version}`;
      return {
        title: "Hermes is running, but not as a service",
        body: `${running} is answering on port ${String(status.port)}, but it was started by hand, so it won't come back after a crash or a restart. Quit the copy you started (press Ctrl-C in its Terminal window), then repair the service. Your bots pause until the service starts them again.`,
        action: "Repair the service",
      };
    }
    case "broken":
      return {
        title: "The Hermes service isn't running",
        body: "The service is installed, but its program is missing or stopped. Repairing reinstalls it from this app. Your data stays where it is.",
        action: "Repair the service",
      };
  }
}
