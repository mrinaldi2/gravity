import { useEffect } from "react";
import type { ReactElement } from "react";
import type { Selection } from "../../app/selection";
import { connectionStatusLabel } from "../../protocol/connection";
import type { ConnectionStatus, Endpoint } from "../../protocol/connection";
import type { SettingsCategory } from "../settings/categories";

interface RailProps {
  readonly selection: Selection;
  /** Everything waiting for the owner, for the Needs you badge. */
  readonly needsYou: number;
  readonly status: ConnectionStatus;
  readonly endpoint: Endpoint;
  /** False on a read-only connection, which the status says. */
  readonly canControl: boolean;
  readonly onSelect: (next: Selection) => void;
  readonly onOpenSettings: (category?: SettingsCategory) => void;
}

/** The Hermes service's state as a dot, opening the connection settings. */
function Connection(
  props: Pick<RailProps, "status" | "endpoint" | "canControl" | "onOpenSettings">,
): ReactElement {
  const { status, endpoint } = props;
  const readOnly = status === "connected" && !props.canControl ? " · read-only" : "";
  const label = `Hermes service: ${connectionStatusLabel(status)}${readOnly} (${endpoint.host}:${endpoint.port})`;
  return (
    <button
      type="button"
      className="rail-item rail-conn"
      title={label}
      aria-label={label}
      onClick={() => {
        props.onOpenSettings("connection");
      }}
    >
      <span className={`conn-dot conn-${status}`} aria-hidden="true" />
    </button>
  );
}

interface RailItemProps {
  readonly glyph: string;
  readonly label: string;
  readonly shortcut: string;
  readonly active: boolean;
  readonly badge?: number;
  readonly onClick: () => void;
}

function RailItem(props: RailItemProps): ReactElement {
  const badge = props.badge ?? 0;
  return (
    <button
      type="button"
      className={`rail-item${props.active ? " rail-item-on" : ""}`}
      aria-current={props.active ? "page" : undefined}
      title={`${props.label} (${props.shortcut})`}
      onClick={props.onClick}
    >
      <span className="rail-glyph" aria-hidden="true">
        {props.glyph}
      </span>
      {props.label}
      {badge > 0 ? (
        <span className="rail-badge" aria-label={`${badge} waiting`}>
          {badge}
        </span>
      ) : null}
    </button>
  );
}

/** ⌘0 opens Projects and ⌘⇧N opens Needs you, from anywhere (UX-024 §2). */
function useRailKeys(onSelect: (next: Selection) => void): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey) {
        return;
      }
      if (event.key === "0" && !event.shiftKey) {
        event.preventDefault();
        onSelect({ kind: "home" });
      } else if (event.shiftKey && event.key.toLowerCase() === "n") {
        event.preventDefault();
        onSelect({ kind: "control" });
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
    };
  }, [onSelect]);
}

/** The app's left rail: Projects · Needs you, with Settings at the bottom. */
export default function Rail(props: RailProps): ReactElement {
  const { selection, needsYou, onSelect, onOpenSettings } = props;
  useRailKeys(onSelect);
  return (
    <nav className="rail" aria-label="Main">
      <RailItem
        glyph="⌂"
        label="Projects"
        shortcut="⌘0"
        active={selection.kind === "home" || selection.kind === "project"}
        onClick={() => {
          onSelect({ kind: "home" });
        }}
      />
      <RailItem
        glyph="◆"
        label="Needs you"
        shortcut="⌘⇧N"
        active={selection.kind === "control"}
        badge={needsYou}
        onClick={() => {
          onSelect({ kind: "control" });
        }}
      />
      <span className="rail-spacer" />
      <Connection
        status={props.status}
        endpoint={props.endpoint}
        canControl={props.canControl}
        onOpenSettings={onOpenSettings}
      />
      <RailItem
        glyph="⚙"
        label="Settings"
        shortcut="⌘,"
        active={false}
        onClick={() => {
          onOpenSettings();
        }}
      />
    </nav>
  );
}
