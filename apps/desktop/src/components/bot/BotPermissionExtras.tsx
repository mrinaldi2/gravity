import { useState } from "react";
import type { ReactElement } from "react";
import type { DaemonApi } from "../../protocol/api";
import type { Bot, NotifyLevel, PermissionExtra } from "../../protocol/entities";
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
];

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
  if (!client.capabilities.includes("permission_profiles") || bot.peer != null) {
    return null;
  }
  const owner = client.hasGrant("approve");
  const granted = bot.permission_extras ?? [];

  const toggle = async (extra: PermissionExtra, on: boolean): Promise<void> => {
    const next = on ? [...granted, extra] : granted.filter((e) => e !== extra);
    setSaving(true);
    try {
      const reply = await client.request(
        { type: "set_bot_permission_extras", bot_id: bot.id, extras: next },
        "bot",
      );
      onBotUpdated(reply.bot);
      onToast("info", "Permissions changed", `${reply.bot.name} is restarting with them.`);
    } catch (error) {
      onToast("error", "Couldn't change the bot's permissions", errText(error));
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
      <span className="field-hint">
        {owner
          ? "Used in the Trusted and Full profiles. Changing them restarts the bot."
          : "Only the owner can change these."}
      </span>
    </fieldset>
  );
}
