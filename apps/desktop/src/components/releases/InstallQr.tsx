import type { ReactElement } from "react";
import { encode } from "uqr";

/** A QR code for `text`, drawn as one SVG path on a white tile (UX-043 §2). */
export default function InstallQr({
  text,
  label,
  size = 120,
}: {
  readonly text: string;
  readonly label: string;
  readonly size?: number;
}): ReactElement {
  const { data } = encode(text, { ecc: "M", border: 2 });
  const cells = data.length;
  const path = data
    .flatMap((row, y) => row.map((dark, x) => (dark ? `M${x} ${y}h1v1h-1z` : "")))
    .join("");
  return (
    <svg
      className="install-qr-code"
      role="img"
      aria-label={label}
      width={size}
      height={size}
      viewBox={`0 0 ${cells} ${cells}`}
      shapeRendering="crispEdges"
    >
      <rect width={cells} height={cells} fill="#fff" />
      <path d={path} fill="#000" />
    </svg>
  );
}
