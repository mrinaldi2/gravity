import { invoke } from "@tauri-apps/api/core";
import { confirmHomeMigration, InstallCancelled } from "./app/homeMigration";
import { isLocalEndpoint } from "./protocol/connection";
import type { Endpoint } from "./protocol/connection";
import { DEFAULT_ENDPOINT } from "./settings";
import { isTauri } from "./tauri";

export interface DaemonHealth {
  readonly status: string;
  readonly version: string;
  /** How the daemon's build knows this app (H-114); absent before 0.16.2. */
  readonly identity?: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/**
 * Asks the Tauri shell to probe the daemon's `/health` endpoint. Runs in Rust
 * because the webview origin cannot make cross-origin requests to the daemon.
 * `null` means nothing answered (or we are outside the Tauri shell).
 */
export async function probeDaemon(endpoint: Endpoint): Promise<DaemonHealth | null> {
  if (!isTauri()) {
    return null;
  }
  try {
    const result: unknown = await invoke("daemon_health", {
      host: endpoint.host,
      port: endpoint.port,
    });
    if (
      isRecord(result) &&
      typeof result["status"] === "string" &&
      typeof result["version"] === "string"
    ) {
      const identity = result["identity"];
      return typeof identity === "string"
        ? { status: result["status"], version: result["version"], identity }
        : { status: result["status"], version: result["version"] };
    }
  } catch {
    // fall through to null
  }
  return null;
}

/**
 * The endpoint the daemon on this machine actually serves on. An install
 * predating the 49777 default keeps serving on its configured port across an
 * upgrade, so the wizard asks the daemon's config instead of assuming the
 * current default. Falls back to the default outside the Tauri shell.
 */
export async function localDaemonEndpoint(): Promise<Endpoint> {
  if (!isTauri()) {
    return DEFAULT_ENDPOINT;
  }
  try {
    const port: unknown = await invoke("local_daemon_port");
    if (typeof port === "number" && Number.isInteger(port) && port > 0) {
      return { host: DEFAULT_ENDPOINT.host, port };
    }
  } catch {
    // fall through to the default
  }
  return DEFAULT_ENDPOINT;
}

/**
 * The daemon's most recent stderr, for explaining an install that never came
 * up. Empty when there is nothing to show.
 */
export async function daemonLogTail(): Promise<string> {
  if (!isTauri()) {
    return "";
  }
  try {
    const tail: unknown = await invoke("daemon_log_tail");
    return typeof tail === "string" ? tail : "";
  } catch {
    return "";
  }
}

/**
 * The dry-run summary of the home migration this machine's install would
 * run, or `null` when it migrates nothing.
 */
async function homeMigrationSummary(): Promise<string | null> {
  const summary: unknown = await invoke("home_migration_summary");
  return typeof summary === "string" ? summary : null;
}

/**
 * Installs and starts the bundled daemon as a launchd user agent on this
 * machine. An install that would move `~/.gravity` asks first and throws
 * {@link InstallCancelled} when declined. Throws with a human-readable
 * message on failure.
 */
export async function installLocalDaemon(currentVersion?: string): Promise<void> {
  let confirmedMigration = false;
  try {
    const summary = await homeMigrationSummary();
    if (summary !== null) {
      if (!(await confirmHomeMigration(summary))) {
        throw new InstallCancelled();
      }
      confirmedMigration = true;
    }
    const args = {
      ...(currentVersion === undefined ? {} : { currentVersion }),
      ...(confirmedMigration ? { confirmedMigration } : {}),
    };
    if (Object.keys(args).length === 0) {
      await invoke("install_local_daemon");
    } else {
      await invoke("install_local_daemon", args);
    }
  } catch (error) {
    if (error instanceof InstallCancelled) {
      throw error;
    }
    throw new Error(typeof error === "string" ? error : "daemon install failed", {
      cause: error,
    });
  }
}

/**
 * Whether the daemon at `endpoint` is the app-managed launchd agent on this
 * machine — the only one we can restart. False for a remote daemon, for the
 * workspace-private daemon `scripts/dev.sh` starts, and outside the Tauri
 * shell.
 */
export async function isManagedLocalDaemon(endpoint: Endpoint): Promise<boolean> {
  if (!isTauri() || !isLocalEndpoint(endpoint)) {
    return false;
  }
  try {
    const managed: unknown = await invoke("local_daemon_is_managed", { port: endpoint.port });
    return managed === true;
  } catch {
    return false;
  }
}

/** What `hermesd service status --json` adds up to; see `daemon/status.rs`. */
export type ServiceState =
  | "healthy"
  | "not_installed"
  | "legacy_only"
  | "unmanaged"
  | "migration_pending"
  | "migrated_service_missing"
  | "broken";

const SERVICE_STATES: readonly string[] = [
  "healthy",
  "not_installed",
  "legacy_only",
  "unmanaged",
  "migration_pending",
  "migrated_service_missing",
  "broken",
] satisfies readonly ServiceState[];

function isServiceState(value: unknown): value is ServiceState {
  return typeof value === "string" && SERVICE_STATES.includes(value);
}

export interface ServiceStatus {
  readonly state: ServiceState;
  readonly port: number;
  /** What answers `/health` on `port`, if anything. */
  readonly version: string | null;
}

/**
 * The state of the Hermes service on this machine, from the bundled daemon's
 * own `service status`. `null` outside the Tauri shell, for a home set by
 * environment, and when the check itself fails: no answer is never a reason
 * to offer an install.
 */
export async function localServiceStatus(): Promise<ServiceStatus | null> {
  if (!isTauri()) {
    return null;
  }
  try {
    const result: unknown = await invoke("local_service_status");
    if (isRecord(result) && isServiceState(result["state"]) && typeof result["port"] === "number") {
      const version = result["version"];
      return {
        state: result["state"],
        port: result["port"],
        version: typeof version === "string" ? version : null,
      };
    }
  } catch {
    // fall through to null
  }
  return null;
}

/**
 * Bounces the launchd agent running the daemon on this machine, stopping every
 * bot session it is running. Throws with a human-readable message on failure.
 */
export async function restartLocalDaemon(): Promise<void> {
  try {
    await invoke("restart_local_daemon");
  } catch (error) {
    throw new Error(typeof error === "string" ? error : "daemon restart failed", {
      cause: error,
    });
  }
}

/**
 * How the wizard reached a daemon: the bundled local one it found or installed,
 * or an address the user typed, optionally with a device token.
 */
export type SetupConnection =
  | { readonly method: "local"; readonly endpoint: Endpoint }
  | { readonly method: "remote"; readonly endpoint: Endpoint; readonly token?: string };
