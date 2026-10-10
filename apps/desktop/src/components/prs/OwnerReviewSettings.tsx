// Project Settings › Owner review (UX-051 "setting", decision 8; H-269):
// which pull requests need your review. Every pull request is the default;
// Some areas picks from the areas reviewers.toml names on main; Only flagged
// waits for a reviewer or the lead to flag one; None leaves it to the
// reviewers. Security work needs you under any choice but None (§16).
// Saving is the owner's: it goes over the app's own connection.

import { useEffect, useRef, useState } from "react";
import type { ReactElement } from "react";
import type { OwnerReviewMode } from "../../protocol/prOwner";
import { MODE_CHOICES, modeOf } from "./ownerText";
import type { PrClient } from "./usePrOwner";
import { useReviewSettings } from "./usePrOwner";

export default function OwnerReviewSettings(props: {
  readonly client: PrClient;
  readonly projectId: string;
  readonly connected: boolean;
  /** Opened from a PR's "Change": focus this panel's heading. */
  readonly focus?: boolean;
}): ReactElement {
  const { settings, error, save } = useReviewSettings(
    props.client,
    props.projectId,
    props.connected,
  );
  const [mode, setMode] = useState<OwnerReviewMode | null>(null);
  const [areas, setAreas] = useState<readonly string[] | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const focus = props.focus === true;
  useEffect(() => {
    if (focus) {
      heading.current?.focus();
      heading.current?.scrollIntoView?.({ block: "start" });
    }
  }, [focus]);

  const owner = props.client.hasGrant("approve");
  const current = settings === null ? null : modeOf(settings);
  const shownMode = mode ?? current ?? "all";
  const shownAreas = areas ?? settings?.ownerReviewAreas ?? [];
  const dirty =
    settings !== null &&
    (shownMode !== current || shownAreas.join("\n") !== settings.ownerReviewAreas.join("\n"));
  const toggle = (area: string): void => {
    setSaved(false);
    setAreas(
      shownAreas.includes(area) ? shownAreas.filter((a) => a !== area) : [...shownAreas, area],
    );
  };
  const submit = async (): Promise<void> => {
    setSaving(true);
    const done = await save({
      type: "review_settings_set",
      project_id: props.projectId,
      owner_review: shownMode,
      owner_review_areas: shownMode === "areas" ? shownAreas : [],
    });
    setSaving(false);
    if (done) {
      setMode(null);
      setAreas(null);
      setSaved(true);
    }
  };

  return (
    <form
      className="panel owner-review-settings"
      aria-labelledby="owner-review-title"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <h3 className="panel-title" id="owner-review-title" tabIndex={-1} ref={heading}>
        Owner review
      </h3>
      <fieldset
        className="field"
        disabled={!props.connected || !owner || saving || settings === null}
      >
        <legend className="field-label">Which pull requests need your review?</legend>
        <span className="field-hint">
          Reviewers always review. This decides when you are a reviewer too. You approve every
          release either way.
        </span>
        {MODE_CHOICES.map((choice) => (
          <div key={choice.mode}>
            <label className="radio-row">
              <input
                type="radio"
                name="owner-review"
                value={choice.mode}
                checked={shownMode === choice.mode}
                onChange={() => {
                  setSaved(false);
                  setMode(choice.mode);
                }}
              />
              <span>
                {choice.title}
                <span className="field-hint">{choice.hint}</span>
              </span>
            </label>
            {choice.mode === "areas" && shownMode === "areas" ? (
              <div className="owner-review-areas" role="group" aria-label="Areas">
                {(settings?.areas ?? []).length === 0 ? (
                  <span className="field-hint">reviewers.toml on main names no areas yet.</span>
                ) : (
                  (settings?.areas ?? []).map((area) => (
                    <label key={area} className="checkbox-row">
                      <input
                        type="checkbox"
                        checked={shownAreas.includes(area)}
                        onChange={() => toggle(area)}
                      />
                      <span>{area}</span>
                    </label>
                  ))
                )}
              </div>
            ) : null}
          </div>
        ))}
      </fieldset>
      <span className="field-hint">
        Security and permissions work needs you under every choice but None.
      </span>
      {owner ? null : (
        <span className="field-hint">
          Only the owner can change this, from a connection with approve access.
        </span>
      )}
      {error ? (
        <span className="field-hint pr-tone-bad" role="alert">
          {error}
        </span>
      ) : null}
      <div className="panel-actions">
        {saved && !dirty ? (
          <span className="field-hint" role="status">
            Saved.
          </span>
        ) : null}
        <button
          type="submit"
          className="btn btn-primary"
          disabled={!props.connected || !owner || saving || !dirty}
        >
          {saving ? "Saving…" : "Save"}
        </button>
      </div>
    </form>
  );
}
