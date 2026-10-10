// Project Settings › Owner review (UX-051 "setting", decision 8; H-269):
// which pull requests need your review. Every pull request is the default;
// Some areas picks from the areas reviewers.toml names on main; Only flagged
// waits for a reviewer or the lead to flag one; None leaves it to the
// reviewers. Security work needs you under any choice but None (§16).
// Saving is the owner's: it goes over the app's own connection.

import { useEffect, useRef, useState } from "react";
import type { ReactElement } from "react";
import type { ReviewSettings } from "../../protocol/gen/hermes/pr/v1/pr_pb";
import type { OwnerReviewMode } from "../../protocol/prOwner";
import { MODE_CHOICES, modeOf } from "./ownerText";
import type { PrClient } from "./usePrOwner";
import { useReviewSettings } from "./usePrOwner";

interface Draft {
  readonly mode: OwnerReviewMode;
  readonly areas: readonly string[];
}

/** The draft the form shows: your edits over what's saved. */
function shown(settings: ReviewSettings | null, edit: Partial<Draft>): Draft {
  return {
    mode: edit.mode ?? (settings === null ? "all" : modeOf(settings)),
    areas: edit.areas ?? settings?.ownerReviewAreas ?? [],
  };
}

function differs(settings: ReviewSettings | null, draft: Draft): boolean {
  if (settings === null) {
    return false;
  }
  return (
    draft.mode !== modeOf(settings) ||
    draft.areas.join("\n") !== settings.ownerReviewAreas.join("\n")
  );
}

function AreaPicker(props: {
  readonly areas: readonly string[];
  readonly picked: readonly string[];
  readonly onToggle: (area: string) => void;
}): ReactElement {
  return (
    <div className="owner-review-areas" role="group" aria-label="Areas">
      {props.areas.length === 0 ? (
        <span className="field-hint">reviewers.toml on main names no areas yet.</span>
      ) : (
        props.areas.map((area) => (
          <label key={area} className="checkbox-row">
            <input
              type="checkbox"
              checked={props.picked.includes(area)}
              onChange={() => props.onToggle(area)}
            />
            <span>{area}</span>
          </label>
        ))
      )}
    </div>
  );
}

function Choices(props: {
  readonly draft: Draft;
  readonly areas: readonly string[];
  readonly onMode: (mode: OwnerReviewMode) => void;
  readonly onToggle: (area: string) => void;
}): ReactElement {
  return (
    <>
      {MODE_CHOICES.map((choice) => (
        <div key={choice.mode}>
          <label className="radio-row">
            <input
              type="radio"
              name="owner-review"
              value={choice.mode}
              checked={props.draft.mode === choice.mode}
              onChange={() => props.onMode(choice.mode)}
            />
            <span>
              {choice.title}
              <span className="field-hint">{choice.hint}</span>
            </span>
          </label>
          {choice.mode === "areas" && props.draft.mode === "areas" ? (
            <AreaPicker areas={props.areas} picked={props.draft.areas} onToggle={props.onToggle} />
          ) : null}
        </div>
      ))}
    </>
  );
}

/** Focuses the panel's heading when asked to, as a PR's "Change" does. */
function useFocusOn(focus: boolean) {
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    if (focus) {
      heading.current?.focus();
      heading.current?.scrollIntoView?.({ block: "start" });
    }
  }, [focus]);
  return heading;
}

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
  const [edit, setEdit] = useState<Partial<Draft>>({});
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const heading = useFocusOn(props.focus === true);
  const draft = shown(settings, edit);
  const dirty = differs(settings, draft);
  const owner = props.client.hasGrant("approve");
  const off = !props.connected || !owner || saving;
  const change = (next: Partial<Draft>): void => {
    setSaved(false);
    setEdit({ ...edit, ...next });
  };
  const toggle = (area: string): void =>
    change({
      areas: draft.areas.includes(area)
        ? draft.areas.filter((a) => a !== area)
        : [...draft.areas, area],
    });
  const submit = async (): Promise<void> => {
    setSaving(true);
    const done = await save({
      type: "review_settings_set",
      project_id: props.projectId,
      owner_review: draft.mode,
      owner_review_areas: draft.mode === "areas" ? draft.areas : [],
    });
    setSaving(false);
    if (done) {
      setEdit({});
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
      <fieldset className="field" disabled={off || settings === null}>
        <legend className="field-label">Which pull requests need your review?</legend>
        <span className="field-hint">
          Reviewers always review. This decides when you are a reviewer too. You approve every
          release either way.
        </span>
        <Choices
          draft={draft}
          areas={settings?.areas ?? []}
          onMode={(mode) => change({ mode })}
          onToggle={toggle}
        />
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
        <button type="submit" className="btn btn-primary" disabled={off || !dirty}>
          {saving ? "Saving…" : "Save"}
        </button>
      </div>
    </form>
  );
}
