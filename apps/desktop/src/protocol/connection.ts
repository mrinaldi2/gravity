// Connection-level vocabulary shared by the client and the UI.

export const PROTOCOL_VERSION = 2;
export const CLIENT_ID = "desktop/0.1.0";

/**
 * `version_mismatch` is a daemon that rejected the handshake because it speaks
 * a different protocol. It is sticky like `auth_failed` — reconnects continue,
 * so the app recovers once compatible versions are installed — but it needs
 * its own status because retrying alone cannot fix it.
 */
export type ConnectionStatus =
  | "connecting"
  | "connected"
  | "disconnected"
  | "auth_failed"
  | "version_mismatch";

const STATUS_LABEL: Readonly<Record<ConnectionStatus, string>> = {
  connected: "Connected",
  connecting: "Connecting…",
  disconnected: "Disconnected",
  auth_failed: "Token rejected",
  version_mismatch: "Update needed",
};

/** Human-readable form of a status, for the footer and the empty pane. */
export function connectionStatusLabel(status: ConnectionStatus): string {
  return STATUS_LABEL[status];
}

export interface Endpoint {
  readonly host: string;
  readonly port: number;
}

export function isLocalEndpoint(endpoint: Endpoint): boolean {
  return endpoint.host === "127.0.0.1" || endpoint.host === "localhost";
}

/** Error carrying a protocol `error` reply code (or a locally generated one). */
export class DaemonError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "DaemonError";
    this.code = code;
  }
}
