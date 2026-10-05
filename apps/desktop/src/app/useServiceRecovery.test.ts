import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Endpoint } from "../protocol/connection";
import { onHomeMigrationRequest } from "./homeMigration";
import type { AddToast } from "./useToasts";
import { useServiceRecovery } from "./useServiceRecovery";

const invoke = vi.hoisted(() => vi.fn<(command: string, args?: unknown) => Promise<unknown>>());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

vi.mock("../analytics", () => ({
  captureException: vi.fn<(error: unknown, context: string) => void>(),
}));

const LOCAL: Endpoint = { host: "127.0.0.1", port: 49777 };
const REMOTE: Endpoint = { host: "mini", port: 49777 };

const LEGACY_ONLY = { state: "legacy_only", port: 49777, version: null };
const HEALTHY = { state: "healthy", port: 49777, version: "0.15.1" };

afterEach(() => {
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  invoke.mockReset();
});

function inTauri(): void {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
}

/** Answers `local_service_status` from `statuses` in turn, everything else with `rest`. */
function answer(statuses: readonly unknown[], rest: (command: string) => Promise<unknown>): void {
  const queue = [...statuses];
  invoke.mockImplementation((command) =>
    command === "local_service_status" ? Promise.resolve(queue.shift() ?? null) : rest(command),
  );
}

function setup(endpoint: Endpoint) {
  const addToast = vi.fn<AddToast>();
  const hook = renderHook(() => useServiceRecovery(endpoint, addToast));
  return { ...hook, addToast };
}

describe("useServiceRecovery", () => {
  it("checks a local service once on launch and offers its fix without installing", async () => {
    inTauri();
    answer([LEGACY_ONLY], () => Promise.resolve(null));
    const { result } = setup(LOCAL);
    await waitFor(() => {
      expect(result.current.offer?.action).toBe("Finish the update");
    });
    expect(invoke.mock.calls.map(([command]) => command)).toEqual(["local_service_status"]);
  });

  it("leaves a machine that connects to a remote daemon alone", async () => {
    inTauri();
    answer([LEGACY_ONLY], () => Promise.resolve(null));
    const { result } = setup(REMOTE);
    await act(async () => {
      await Promise.resolve();
    });
    expect(result.current.offer).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("installs without migrating on the owner's click, then clears", async () => {
    inTauri();
    answer([LEGACY_ONLY, HEALTHY], () => Promise.resolve(null));
    const { result, addToast } = setup(LOCAL);
    await waitFor(() => {
      expect(result.current.offer).not.toBeNull();
    });
    act(() => {
      result.current.install();
    });
    await waitFor(() => {
      expect(result.current.offer).toBeNull();
    });
    expect(invoke).toHaveBeenCalledWith("install_local_daemon");
    expect(addToast).toHaveBeenCalledWith("info", "Hermes service running", expect.any(String));
  });

  it("asks before an install that moves the home", async () => {
    inTauri();
    const pending = { state: "migration_pending", port: 49777, version: "0.14.2" };
    answer([pending, HEALTHY], (command) =>
      Promise.resolve(command === "home_migration_summary" ? "Moves ~/.gravity" : null),
    );
    const asked: string[] = [];
    const stop = onHomeMigrationRequest((request) => {
      if (request !== null) {
        asked.push(request.summary);
        request.answer(true);
      }
    });
    const { result } = setup(LOCAL);
    await waitFor(() => {
      expect(result.current.offer?.action).toBe("Finish the update");
    });
    act(() => {
      result.current.install();
    });
    await waitFor(() => {
      expect(result.current.offer).toBeNull();
    });
    stop();
    expect(asked).toEqual(["Moves ~/.gravity"]);
    expect(invoke).toHaveBeenCalledWith("install_local_daemon", {
      currentVersion: "0.14.2",
      confirmedMigration: true,
    });
  });

  it("keeps the offer and shows the installer's error verbatim", async () => {
    inTauri();
    const failure =
      "daemon install failed: Error: the old home is still in use\n  pid 7 python3 (cwd /u/.gravity/serve)";
    answer([LEGACY_ONLY], (command) =>
      command === "install_local_daemon" ? Promise.reject(failure) : Promise.resolve(null),
    );
    const { result } = setup(LOCAL);
    await waitFor(() => {
      expect(result.current.offer).not.toBeNull();
    });
    act(() => {
      result.current.install();
    });
    await waitFor(() => {
      expect(result.current.error).toBe(failure);
    });
    expect(result.current.offer?.action).toBe("Finish the update");
    expect(result.current.installing).toBe(false);
  });

  it("hides the offer when dismissed", async () => {
    inTauri();
    answer([LEGACY_ONLY], () => Promise.resolve(null));
    const { result } = setup(LOCAL);
    await waitFor(() => {
      expect(result.current.offer).not.toBeNull();
    });
    act(() => {
      result.current.dismiss();
    });
    expect(result.current.offer).toBeNull();
    expect(invoke).not.toHaveBeenCalledWith("install_local_daemon");
  });
});
