import type { ReactElement } from "react";
import type { RecoveryOffer } from "../../app/serviceRecovery";

interface ServiceRecoveryBannerProps {
  readonly offer: RecoveryOffer;
  readonly installing: boolean;
  /** The installer's error text, shown exactly as it came. */
  readonly error: string | null;
  readonly onInstall: () => void;
  readonly onDismiss: () => void;
}

/**
 * The one fix the launch-time service check found: what is wrong and a
 * single button that runs the install. "Not now" only hides it.
 */
export default function ServiceRecoveryBanner({
  offer,
  installing,
  error,
  onInstall,
  onDismiss,
}: ServiceRecoveryBannerProps): ReactElement {
  return (
    <div className="service-recovery" role="alert">
      <div className="service-recovery-text">
        <div className="service-recovery-title">{offer.title}</div>
        <div className="service-recovery-body">{offer.body}</div>
        {error === null ? null : (
          <pre className="service-recovery-error" aria-label="Installer output">
            {error}
          </pre>
        )}
      </div>
      <div className="service-recovery-actions">
        <button type="button" className="btn btn-small" disabled={installing} onClick={onDismiss}>
          Not now
        </button>
        <button
          type="button"
          className="btn btn-small btn-primary"
          disabled={installing}
          onClick={onInstall}
        >
          {installing ? "Working…" : offer.action}
        </button>
      </div>
    </div>
  );
}
