import { useCallback, useEffect, useState } from "react";
import { captureException } from "../analytics";
import { isLocalEndpoint } from "../protocol/connection";
import type { Endpoint } from "../protocol/connection";
import { installLocalDaemon, localServiceStatus } from "../setup";
import type { ServiceStatus } from "../setup";
import { InstallCancelled } from "./homeMigration";
import { recoveryFor } from "./serviceRecovery";
import type { RecoveryOffer } from "./serviceRecovery";
import type { AddToast } from "./useToasts";

export interface ServiceRecovery {
  /** What to offer, or `null` when the service needs nothing. */
  readonly offer: RecoveryOffer | null;
  readonly installing: boolean;
  /** The installer's own error text from the last attempt, verbatim. */
  readonly error: string | null;
  readonly install: () => void;
  readonly dismiss: () => void;
}

/** How an offered install ended; `error` is the installer's own text. */
export type InstallOutcome = "installed" | "cancelled" | { readonly error: string };

/**
 * Installs the service for `status` after the owner's click: `service
 * install`, which asks before a migration and runs `--no-migrate`
 * otherwise.
 */
export async function installForRecovery(status: ServiceStatus): Promise<InstallOutcome> {
  try {
    await installLocalDaemon(status.version ?? undefined);
    return "installed";
  } catch (error) {
    if (error instanceof InstallCancelled) {
      return "cancelled";
    }
    captureException(error, "daemon_install");
    return { error: error instanceof Error ? error.message : "install failed" };
  }
}

/**
 * Checks the Hermes service on this machine once on launch, when the app
 * talks to a local daemon, and holds the one fix to offer. Nothing installs
 * until the owner clicks.
 */
export function useServiceRecovery(endpoint: Endpoint, addToast: AddToast): ServiceRecovery {
  const local = isLocalEndpoint(endpoint);
  const [status, setStatus] = useState<ServiceStatus | null>(null);
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!local) {
      return undefined;
    }
    let disposed = false;
    const check = async (): Promise<void> => {
      const next = await localServiceStatus();
      if (!disposed) {
        // oxlint-disable-next-line react/set-state-in-effect
        setStatus(next);
      }
    };
    void check();
    return (): void => {
      disposed = true;
    };
  }, [local]);

  const install = useCallback((): void => {
    if (status === null) {
      return;
    }
    setInstalling(true);
    setError(null);
    void (async (): Promise<void> => {
      const outcome = await installForRecovery(status);
      const next = outcome === "installed" ? await localServiceStatus() : status;
      setInstalling(false);
      setError(typeof outcome === "object" ? outcome.error : null);
      setStatus(next);
      if (outcome === "installed" && (next === null || recoveryFor(next) === null)) {
        addToast("info", "Hermes service running", "The app reconnects on its own.");
      }
    })();
  }, [addToast, status]);

  const dismiss = useCallback((): void => {
    setStatus(null);
    setError(null);
  }, []);

  const offer = local && status !== null ? recoveryFor(status) : null;
  return { offer, installing, error, install, dismiss };
}
