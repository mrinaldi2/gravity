import { useState } from "react";
import type { ReactElement } from "react";
import type { DaemonApi } from "../protocol/api";
import type { NotifyLevel, PermissionProfile, Project } from "../protocol/entities";
import { errText } from "../util";
import ConfirmDialog from "./overlay/ConfirmDialog";

interface ProjectPermissionsFormProps {
  readonly client: DaemonApi;
  readonly project: Project;
  readonly connected: boolean;
  readonly onToast: (level: NotifyLevel, title: string, body: string) => void;
}

const PROFILES: readonly {
  readonly id: PermissionProfile;
  readonly name: string;
  readonly help: string;
  /** Shown but not selectable, with this line saying why. */
  readonly unavailable?: string;
}[] = [
  {
    id: "standard",
    name: "Standard",
    help: "Bots check with you before anything Claude Code's auto mode is unsure about.",
  },
  {
    id: "trusted",
    name: "Trusted",
    help: "Recommended. Builds, tests, worktrees and feature-branch work run without asking; anything unusual is still reviewed first.",
  },
  {
    id: "full",
    name: "Full",
    help: "Nothing is reviewed: only the hard deny-list and the guard stop a bot. The guard refuses inline interpreter code and anything outside the bot's own folders, but can't read script files. For bots in a container, VM or separate user account.",
    // CE-004 (b): the daemon refuses it too; a project stored as Full runs as Trusted.
    unavailable:
      "Not available yet: bots run as you, and if the guard stops running nothing else would review them.",
  },
];

/** What the owner reads before Full is applied (H-031 §3). */
const FULL_WARNING =
  "In Full, nothing reviews what a bot runs except the deny-list and the guard. The guard reads every command line, but not the script files a bot writes and runs. Bots run as you, with your SSH keys, logins, browser sessions and files, so a web page a bot reads could make it act on any of them. Use Full only when this project's bots run in a container, a VM or a separate user account.";

/**
 * How much the project's bots may do without asking (H-031). Owner only: a
 * connection without the approve grant sees the profile but can't change it.
 * Changing it restarts the project's bots, so the change is confirmed.
 */
export default function ProjectPermissionsForm({
  client,
  project,
  connected,
  onToast,
}: ProjectPermissionsFormProps): ReactElement | null {
  const current = project.permission_profile ?? "standard";
  const [chosen, setChosen] = useState<PermissionProfile>(current);
  const [confirming, setConfirming] = useState(false);
  const [saving, setSaving] = useState(false);
  if (!client.capabilities.includes("permission_profiles")) {
    return null;
  }
  const owner = client.hasGrant("approve");
  const picked = PROFILES.find((p) => p.id === chosen) ?? PROFILES[0];

  const apply = async (): Promise<void> => {
    setConfirming(false);
    setSaving(true);
    try {
      await client.request(
        { type: "set_project_permission_profile", project_id: project.id, profile: chosen },
        "project",
      );
      onToast("info", `${picked?.name ?? chosen} profile on`, "The project's bots are restarting.");
    } catch (error) {
      onToast("error", "Couldn't change the permission profile", errText(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <form
      className="panel"
      onSubmit={(event) => {
        event.preventDefault();
        if (chosen !== current) {
          setConfirming(true);
        }
      }}
    >
      <h3 className="panel-title">Permissions</h3>
      <fieldset className="field" disabled={!connected || !owner || saving}>
        <legend className="field-label">What bots may do without asking</legend>
        {PROFILES.map((profile) => (
          <label key={profile.id} className="radio-row">
            <input
              type="radio"
              name="permission-profile"
              value={profile.id}
              checked={chosen === profile.id}
              disabled={profile.unavailable !== undefined}
              onChange={() => {
                setChosen(profile.id);
              }}
            />
            <span>
              {profile.name}
              <span className="field-hint">{profile.help}</span>
              {profile.unavailable === undefined ? null : (
                <span className="field-hint">{profile.unavailable}</span>
              )}
            </span>
          </label>
        ))}
      </fieldset>
      {owner ? null : (
        <span className="field-hint">
          Only the owner can change this, from a connection with approve access.
        </span>
      )}
      <div className="panel-actions">
        <button
          type="submit"
          className="btn btn-primary"
          disabled={!connected || !owner || saving || chosen === current}
        >
          {saving ? "Applying…" : "Apply profile"}
        </button>
      </div>
      {confirming ? (
        <ConfirmDialog
          title={`Switch ${project.name} to ${picked?.name ?? chosen}?`}
          body={`${chosen === "full" ? `${FULL_WARNING} ` : ""}Every bot in this project restarts to pick up the change.`}
          confirmLabel={`Switch to ${picked?.name ?? chosen}`}
          onConfirm={() => {
            void apply();
          }}
          onCancel={() => {
            setConfirming(false);
          }}
        />
      ) : null}
    </form>
  );
}
