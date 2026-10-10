// The owner's writes on a pull request, and the project's Owner review
// setting. Every write is a JSON request on the app's own connection: the
// daemon takes them only from a paired device or the app's ticket (H-269).

import { useCallback, useEffect, useState } from "react";
import type { DaemonApi } from "../../protocol/api";
import type { ReviewSettings } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { PrOwnerRequestBody } from "../../protocol/prOwner";
import { decodeReviewSettings } from "../../protocol/prOwner";
import type { PrApi } from "../../protocol/prs";
import { errText } from "../../util";

/** What the PR views need: the binary reads, and JSON writes as the owner. */
export type PrClient = PrApi & Pick<DaemonApi, "request" | "hasGrant">;

/** The reply each write expects. */
function expected(body: PrOwnerRequestBody): "pr" | "comment" | "review_settings" {
  switch (body.type) {
    case "pr_comment_add":
    case "pr_comment_resolve":
      return "comment";
    case "review_settings_get":
    case "review_settings_set":
      return "review_settings";
    default:
      return "pr";
  }
}

export interface OwnerAct {
  readonly busy: boolean;
  readonly error: string | null;
  /** Sends one write; true when the daemon took it. `after` reads what it changed. */
  readonly act: (body: PrOwnerRequestBody) => Promise<boolean>;
  readonly clear: () => void;
}

export function useOwnerAct(client: PrClient, after: () => void): OwnerAct {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const act = useCallback(
    async (body: PrOwnerRequestBody): Promise<boolean> => {
      setBusy(true);
      setError(null);
      try {
        await client.request(body, expected(body));
        after();
        return true;
      } catch (caught) {
        setError(errText(caught));
        return false;
      } finally {
        setBusy(false);
      }
    },
    [client, after],
  );
  const clear = useCallback(() => setError(null), []);
  return { busy, error, act, clear };
}

export interface SettingsState {
  readonly settings: ReviewSettings | null;
  readonly error: string | null;
  readonly save: (
    body: Extract<PrOwnerRequestBody, { type: "review_settings_set" }>,
  ) => Promise<boolean>;
}

/** The project's Owner review setting, read once per connection. */
export function useReviewSettings(
  client: PrClient,
  projectId: string,
  connected: boolean,
): SettingsState {
  const [settings, setSettings] = useState<ReviewSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!connected) {
      return undefined;
    }
    let live = true;
    client.request({ type: "review_settings_get", project_id: projectId }, "review_settings").then(
      (reply) => live && setSettings(decodeReviewSettings(reply.review_settings)),
      (caught: unknown) => live && setError(errText(caught)),
    );
    return () => {
      live = false;
    };
  }, [client, projectId, connected]);
  const save = useCallback(
    async (body: Extract<PrOwnerRequestBody, { type: "review_settings_set" }>) => {
      try {
        const reply = await client.request(body, "review_settings");
        setSettings(decodeReviewSettings(reply.review_settings));
        setError(null);
        return true;
      } catch (caught) {
        setError(errText(caught));
        return false;
      }
    },
    [client],
  );
  return { settings, error, save };
}
