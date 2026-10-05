// How the daemon's build knows this app (H-114), from its `/health`: the
// same line `hermesd --version` and `service status` print.

import { useEffect, useState } from "react";
import type { Endpoint } from "../../protocol/connection";
import { probeDaemon } from "../../setup";
import { isTauri } from "../../tauri";

/** "signed ABCDE12345", "dev build (phase 2 disabled)", …; "" when unknown. */
export function useDaemonIdentity(endpoint: Endpoint): string {
  const [identity, setIdentity] = useState("");
  useEffect(() => {
    if (!isTauri()) {
      return undefined;
    }
    let disposed = false;
    const probe = async (): Promise<void> => {
      const health = await probeDaemon(endpoint);
      if (!disposed) {
        setIdentity(health?.identity ?? "");
      }
    };
    void probe();
    return (): void => {
      disposed = true;
    };
  }, [endpoint]);
  return identity;
}
