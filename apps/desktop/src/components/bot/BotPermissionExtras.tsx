import { useCallback, useState } from "react";
import type { ReactElement } from "react";
import { useLoadOnConnect } from "../../hooks/useLoadOnConnect";
import type { DaemonApi } from "../../protocol/api";
import type { Bot, BotGrant, NotifyLevel, PermissionExtra } from "../../protocol/entities";
import { errText } from "../../util";

interface BotPermissionExtrasProps {
  readonly client: DaemonApi;
  readonly bot: Bot;
  readonly connected: boolean;
  readonly onBotUpdated: (bot: Bot) => void;
  readonly onToast: (level: NotifyLevel, title: string, body: string) => void;
}

const EXTRAS: readonly {
  readonly id: PermissionExtra;
  readonly label: string;
  readonly help: string;
}[] = [
  {
    id: "publish",
    label: "Publish builds",
    help: "Run its own serve/serve.sh and serve/publish.sh.",
  },
  {
    id: "daemon_restart",
    label: "Restart the Hermes service",
    help: "Restarts every bot, so it must announce it first.",
  },
  { id: "app_restart", label: "Restart its dev app", help: "Run scripts/dev.sh." },
  { id: "install", label: "Install builds", help: "Into Applications or its own simulator." },
  {
    id: "release_main",
    label: "Release to main",
    help: "Push and merge to main, in every profile. Give it to DevOps only.",
  },
  {
    id: "quiesce",
    label: "Pause all projects for an install",
    help: "Holds every project on this computer still while it installs an approved release, then resumes them. Give it to DevOps and the testers that install.",
  },
  {
    id: "build_installers",
    label: "Build the Windows installer",
    help: "Runs the repo's installer build for an approved release, only as committed. Give it to Tester Win.",
  },
];

/** The extras rulings on linked computers granted the bot, read on connect. */
function useGrants(client: DaemonApi, botId: string, connected: boolean): readonly BotGrant[] {
  const [grants, setGrants] = useState<readonly BotGrant[]>([]);
  const load = useCallback(async (): Promise<void> => {
    try {
      const reply = await client.request({ type: "bot_grants", bot_id: botId }, "bot_grants");
      setGrants(reply.grants);
    } catch {
      // An older daemon keeps no record of them.
      setGrants([]);
    }
  }, [client, botId]);
  useLoadOnConnect(connected, load);
  return grants;
}

function label(extra: PermissionExtra): string {
  return EXTRAS.find((e) => e.id === extra)?.label ?? extra;
}

/** "Granted from mac by a ruling: Install builds · 6 Oct, 10:02 · Let …". */
function GrantList({ grants }: { readonly grants: readonly BotGrant[] }): ReactElement | null {
  if (grants.length === 0) {
    return null;
  }
  return (
    <ul className="field-hint bot-grants" aria-label="Granted by rulings on linked computers">
      {grants.map((g) => (
        <li key={`${g.at}-${g.from}`}>
          Granted from {g.from} by a ruling: {g.extras.map(label).join(", ")} ·{" "}
          {new Date(g.at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })}
          {g.decision ? ` · ${g.decision}` : ""}
        </li>
      ))}
    </ul>
  );
}

/**
 * Powers one bot gets on top of its project's permission profile (H-031),
 * such as DevOps publishing builds. They apply in the Trusted and Full
 * profiles. Owner only; changing them restarts the bot.
 */
export default function BotPermissionExtras({
  client,
  bot,
  connected,
  onBotUpdated,
  onToast,
}: BotPermissionExtrasProps): ReactElement | null {
  const [saving, setSaving] = useState(false);
  // A refused change stays visible next to the boxes it was about (H-039).
  const [failure, setFailure] = useState<string | null>(null);
  const grants = useGrants(client, bot.id, connected);
  if (!client.capabilities.includes("permission_profiles") || bot.peer != null) {
    return null;
  }
  const owner = client.hasGrant("approve");
  const granted = bot.permission_extras ?? [];

  const toggle = async (extra: PermissionExtra, on: boolean): Promise<void> => {
    const next = on ? [...granted, extra] : granted.filter((e) => e !== extra);
    setSaving(true);
    setFailure(null);
    try {
      const reply = await client.request(
        { type: "set_bot_permission_extras", bot_id: bot.id, extras: next },
        "bot",
      );
      onBotUpdated(reply.bot);
      onToast("info", "Permissions changed", `${reply.bot.name} is restarting with them.`);
    } catch (error) {
      setFailure(`Couldn't change the bot's permissions: ${errText(error)}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <fieldset className="field bot-permission-extras" disabled={!connected || !owner || saving}>
      <legend className="field-label">Extra permissions</legend>
      {EXTRAS.map((extra) => (
        <label key={extra.id} className="checkbox-row">
          <input
            type="checkbox"
            checked={granted.includes(extra.id)}
            onChange={(event) => void toggle(extra.id, event.target.checked)}
          />
          <span>
            {extra.label}
            <span className="field-hint">{extra.help}</span>
          </span>
        </label>
      ))}
      {failure != null && (
        <p className="field-error" role="alert">
          {failure}
        </p>
      )}
      <GrantList grants={grants} />
      <span className="field-hint">
        {owner
          ? "Used in the Trusted and Full profiles (Release to main in every profile). Changing them restarts the bot."
          : "Only the owner can change these."}
      </span>
    </fieldset>
  );
}
