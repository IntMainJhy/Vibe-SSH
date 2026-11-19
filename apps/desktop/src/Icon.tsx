import type { CSSProperties } from "react";

const paths = {
  up: <path d="M12 19V5m-6 6 6-6 6 6" />,
  history: <path d="M3 11a9 9 0 1 1 2.5 7M3 4v7h7M12 7v5l3 2" />,
  rename: <path d="M4 7h9M8 4v16M5 4h6M5 20h6M16 9h5M16 15h5M18.5 7v10" />,
  folder: <path d="M3 6a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v10H3z" />,
  file: (
    <>
      <path d="M5 3h9l5 5v13H5zM14 3v6h5M8 13h8M8 17h6" />
    </>
  ),
  upload: <path d="M12 16V3m-5 5 5-5 5 5M4 15v6h16v-6" />,
  download: <path d="M12 3v13m-5-5 5 5 5-5M4 15v6h16v-6" />,
  edit: <path d="m15 4 5 5-11 11H4v-5zM13 6l5 5" />,
  trash: <path d="M3 6h18M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7M14 10v7" />,
  home: <path d="m3 11 9-8 9 8M5 10v11h5v-7h4v7h5V10" />,
  activity: <path d="M2 12h5l3-8 4 16 3-8h5" />,
  terminal: (
    <>
      <path d="m5 6 5 6-5 6M13 18h6" />
    </>
  ),
  server: (
    <>
      <rect x="3" y="3" width="18" height="7" rx="2" />
      <rect x="3" y="14" width="18" height="7" rx="2" />
      <path d="M7 6.5h.01M7 17.5h.01M11 6.5h6M11 17.5h6" />
    </>
  ),
  grid: (
    <>
      <rect x="3" y="3" width="7" height="7" rx="1.5" />
      <rect x="14" y="3" width="7" height="7" rx="1.5" />
      <rect x="3" y="14" width="7" height="7" rx="1.5" />
      <rect x="14" y="14" width="7" height="7" rx="1.5" />
    </>
  ),
  plus: <path d="M12 5v14M5 12h14" />,
  search: (
    <>
      <circle cx="10.5" cy="10.5" r="6.5" />
      <path d="m16 16 4.5 4.5" />
    </>
  ),
  chevron: <path d="m9 5 7 7-7 7" />,
  settings: (
    <>
      <path d="M4 7h16M4 17h16" />
      <circle cx="9" cy="7" r="3" />
      <circle cx="15" cy="17" r="3" />
    </>
  ),
  close: <path d="m6 6 12 12M6 18 18 6" />,
  check: <path d="m5 12 4 4L19 6" />,
  copy: (
    <>
      <rect x="8" y="8" width="13" height="13" rx="2" />
      <path d="M16 4V3H3v13h1" />
    </>
  ),
  refresh: (
    <>
      <path d="M20 7v5h-5M4 17v-5h5" />
      <path d="M5.3 7a8 8 0 0 1 13.2-2L20 7M4 17l1.5 2A8 8 0 0 0 18.7 17" />
    </>
  ),
  shield: (
    <>
      <path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6z" />
      <path d="m8.5 11.5 2.5 2.5 4.5-5" />
    </>
  ),
  link: (
    <>
      <path d="m10 13 4-4M8 16l-1 1a3.5 3.5 0 0 1-5-5l4-4a3.5 3.5 0 0 1 5 0M16 8l1-1a3.5 3.5 0 0 1 5 5l-4 4a3.5 3.5 0 0 1-5 0" />
    </>
  ),
  stop: <rect x="5" y="5" width="14" height="14" rx="2" />,
  arrow: <path d="M4 12h16m-6-6 6 6-6 6" />,
  back: <path d="M20 12H4m6-6-6 6 6 6" />,
  clock: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 2" />
    </>
  ),
  alert: (
    <>
      <path d="m12 3 10 18H2zM12 9v4M12 17h.01" />
    </>
  ),
  sun: (
    <>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 1v2M12 21v2M1 12h2M21 12h2M4 4l1.5 1.5M18.5 18.5 20 20M4 20l1.5-1.5M18.5 5.5 20 4" />
    </>
  ),
  moon: <path d="M21 13A9 9 0 0 1 11 3a9 9 0 1 0 10 10Z" />,
};
export type IconName = keyof typeof paths;
export function Icon({
  name,
  size = 18,
  style,
}: {
  name: IconName;
  size?: number;
  style?: CSSProperties;
}) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.65"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      style={style}
    >
      {paths[name]}
    </svg>
  );
}
