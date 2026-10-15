import type { ChannelKind } from "../api/types";

const P = {
  org: <><rect x="5.75" y="1.75" width="4.5" height="3.5" rx="1" /><rect x="1.75" y="10.75" width="4.5" height="3.5" rx="1" /><rect x="9.75" y="10.75" width="4.5" height="3.5" rx="1" /><path d="M8 5.25V8M4 10.75V9a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v1.75" /></>,
  inbox: <><path d="M1.75 9.25h3.5l1 1.75h3.5l1-1.75h3.5" /><path d="M3.6 3.4A1.5 1.5 0 0 1 4.9 2.75h6.2a1.5 1.5 0 0 1 1.3.65l1.85 5.4v3.95a1.5 1.5 0 0 1-1.5 1.5H3.25a1.5 1.5 0 0 1-1.5-1.5V8.8Z" /></>,
  activity: <path d="M1.75 8h2.5l2-4.75 3.5 9.5 2-4.75h2.5" />,
  settings: <><path d="M2.25 4.5h6.5M11.75 4.5h2M2.25 11.5h2M7.25 11.5h6.5" /><circle cx="10.25" cy="4.5" r="1.5" /><circle cx="5.75" cy="11.5" r="1.5" /></>,
  plus: <path d="M8 3v10M3 8h10" />,
  search: <><circle cx="7.25" cy="7.25" r="4.5" /><path d="m10.6 10.6 3.15 3.15" /></>,
  close: <path d="m4 4 8 8M12 4l-8 8" />,
  chevronDown: <path d="m4.5 6.25 3.5 3.5 3.5-3.5" />,
  chevronRight: <path d="m6.25 4.5 3.5 3.5-3.5 3.5" />,
  arrowLeft: <path d="M13 8H3.5M7.5 4 3.5 8l4 4" />,
  arrowRight: <path d="M3 8h9.5M8.5 4l4 4-4 4" />,
  arrowUpRight: <path d="M5 11 11 5M6 5h5v5" />,
  play: <path d="M5 3.6v8.8a.6.6 0 0 0 .9.52l7.1-4.4a.6.6 0 0 0 0-1.04L5.9 3.08a.6.6 0 0 0-.9.52Z" />,
  stop: <rect x="4" y="4" width="8" height="8" rx="1.5" />,
  clock: <><circle cx="8" cy="8" r="6.25" /><path d="M8 4.75V8l2.25 1.5" /></>,
  sparkle: <path d="M8 1.75c.35 2.9 1.35 3.9 4.25 4.25C9.35 6.35 8.35 7.35 8 10.25 7.65 7.35 6.65 6.35 3.75 6 6.65 5.65 7.65 4.65 8 1.75ZM12.5 10.25c.18 1.15.6 1.57 1.75 1.75-1.15.18-1.57.6-1.75 1.75-.18-1.15-.6-1.57-1.75-1.75 1.15-.18 1.57-.6 1.75-1.75Z" />,
  layers: <><path d="m8 1.75 6.25 3.25L8 8.25 1.75 5Z" /><path d="m1.75 8 6.25 3.25L14.25 8M1.75 11 8 14.25 14.25 11" /></>,
  file: <><path d="M9.25 1.75H4.5A1.25 1.25 0 0 0 3.25 3v10a1.25 1.25 0 0 0 1.25 1.25h7A1.25 1.25 0 0 0 12.75 13V5.25Z" /><path d="M9.25 1.75v3.5h3.5" /></>,
  lock: <><rect x="3.25" y="7.25" width="9.5" height="7" rx="1.5" /><path d="M5.25 7.25V5a2.75 2.75 0 0 1 5.5 0v2.25" /></>,
  check: <path d="m3.25 8.5 3 3 6.5-7" />,
  checkCircle: <><circle cx="8" cy="8" r="6.25" /><path d="m5.25 8.25 1.9 1.9 3.6-3.9" /></>,
  alert: <><path d="M7.13 2.5a1 1 0 0 1 1.74 0l5.5 9.75a1 1 0 0 1-.87 1.5H2.5a1 1 0 0 1-.87-1.5Z" /><path d="M8 6.25v3M8 11.5v.01" /></>,
  send: <path d="M8 13V3.5M4 7.5l4-4 4 4" />,
  chat: <path d="M2.25 7.6c0-2.95 2.57-5.35 5.75-5.35s5.75 2.4 5.75 5.35-2.57 5.35-5.75 5.35c-.8 0-1.55-.15-2.25-.42L2.5 13.5l.95-2.6A5.1 5.1 0 0 1 2.25 7.6Z" />,
  calendar: <><rect x="2" y="3.25" width="12" height="10.75" rx="1.75" /><path d="M2 6.75h12M5.25 1.75v3M10.75 1.75v3" /></>,
  user: <><circle cx="8" cy="5.25" r="2.75" /><path d="M2.75 14c.5-2.6 2.6-4.25 5.25-4.25S12.75 11.4 13.25 14" /></>,
  userPlus: <><circle cx="6.5" cy="5.25" r="2.75" /><path d="M1.5 14c.45-2.6 2.45-4.25 5-4.25 1.3 0 2.45.43 3.3 1.17M12.25 8.75v4.5M10 11h4.5" /></>,
  trash: <path d="M2.75 4.25h10.5M6.5 2.25h3M4.25 4.25l.6 8.6a1.25 1.25 0 0 0 1.25 1.15h3.8a1.25 1.25 0 0 0 1.25-1.15l.6-8.6" />,
  edit: <path d="M9.9 3.1 12.9 6.1M2.75 13.25l.6-2.9L10.6 3.1a1.4 1.4 0 0 1 2 0l.3.3a1.4 1.4 0 0 1 0 2l-7.25 7.25Z" />,
  undo: <path d="M5.75 3.5 2.75 6.5l3 3M2.75 6.5h6.5a4 4 0 0 1 0 8H7" />,
  promote: <><path d="M8 10.5V2.75M5 5.75l3-3 3 3" /><path d="M2.75 10.25v2.5c0 .83.67 1.5 1.5 1.5h7.5c.83 0 1.5-.67 1.5-1.5v-2.5" /></>,
  copy: <><rect x="5.25" y="5.25" width="8.5" height="8.5" rx="1.5" /><path d="M10.75 5.25V3.75a1.5 1.5 0 0 0-1.5-1.5h-5.5a1.5 1.5 0 0 0-1.5 1.5v5.5a1.5 1.5 0 0 0 1.5 1.5h1.5" /></>,
  folder: <path d="M1.75 4.25c0-.83.67-1.5 1.5-1.5h2.9l1.5 1.75h5.1c.83 0 1.5.67 1.5 1.5v6.25c0 .83-.67 1.5-1.5 1.5h-9.5c-.83 0-1.5-.67-1.5-1.5Z" />,
  external: <><path d="M9.25 2.25h4.5v4.5M13.75 2.25 7.5 8.5" /><path d="M12 9.5v3a1.25 1.25 0 0 1-1.25 1.25H3.5a1.25 1.25 0 0 1-1.25-1.25V5.25A1.25 1.25 0 0 1 3.5 4h3" /></>,
  moon: <path d="M13.5 9.6A5.75 5.75 0 0 1 6.4 2.5 5.75 5.75 0 1 0 13.5 9.6Z" />,
  sun: <><circle cx="8" cy="8" r="2.75" /><path d="M8 1.5v1.25M8 13.25v1.25M1.5 8h1.25M13.25 8h1.25M3.4 3.4l.9.9M11.7 11.7l.9.9M3.4 12.6l.9-.9M11.7 4.3l.9-.9" /></>,
  monitor: <><rect x="1.75" y="2.5" width="12.5" height="8.75" rx="1.5" /><path d="M5.5 14h5M8 11.25V14" /></>,
  refresh: <><path d="M13.25 3.5v3h-3" /><path d="M13 6.25A5.25 5.25 0 1 0 12.4 11" /></>,
  tool: <path d="M9.9 2.4a3.25 3.25 0 0 0-3.9 4.35L2.4 10.35a1.4 1.4 0 0 0 0 2l1.25 1.25a1.4 1.4 0 0 0 2 0L9.25 10a3.25 3.25 0 0 0 4.35-3.9l-2.05 2.05-2.3-.4-.4-2.3Z" />,
  branch: <><circle cx="4" cy="3.5" r="1.75" /><circle cx="4" cy="12.5" r="1.75" /><circle cx="12" cy="7" r="1.75" /><path d="M4 5.25v5.5M4 8.25c0-.7.55-1.25 1.25-1.25h5" /></>,
  ask: <><path d="M2.25 7.6c0-2.95 2.57-5.35 5.75-5.35s5.75 2.4 5.75 5.35-2.57 5.35-5.75 5.35c-.8 0-1.55-.15-2.25-.42L2.5 13.5l.95-2.6A5.1 5.1 0 0 1 2.25 7.6Z" /><path d="M6.5 6.25a1.5 1.5 0 1 1 2.2 1.33c-.42.22-.7.52-.7.92v.25M8 10.5v.01" /></>,
  mail: <><rect x="1.75" y="3.25" width="12.5" height="9.5" rx="1.5" /><path d="m2.25 4 5.75 4.5L13.75 4" /></>,
  phone: <path d="M3.5 2h2.25l1.2 3-1.6 1.1a8 8 0 0 0 4.55 4.55L11 9.05l3 1.2v2.25a1.5 1.5 0 0 1-1.6 1.5C7.1 13.6 2.4 8.9 2 3.6A1.5 1.5 0 0 1 3.5 2Z" />,
  hash: <path d="M6.25 2.25 5 13.75M11 2.25 9.75 13.75M2.5 5.75h11M2.25 10.25h11" />,
  ticket: <path d="M1.75 5.5V4.25c0-.55.45-1 1-1h10.5c.55 0 1 .45 1 1V5.5a2.5 2.5 0 0 0 0 5v1.25c0 .55-.45 1-1 1H2.75c-.55 0-1-.45-1-1V10.5a2.5 2.5 0 0 0 0-5ZM9.75 3.25v1.5M9.75 7.25v1.5M9.75 11.25v1.5" />,
  checklist: <path d="m2 4.25 1.25 1.25L5.5 3M2 10.25l1.25 1.25L5.5 9M7.75 4.25h6.25M7.75 10.25h6.25" />,
  board: <><rect x="1.75" y="2.25" width="12.5" height="11.5" rx="1.5" /><path d="M6 2.25v11.5M10 2.25v11.5M3.5 5h1M7.5 5h1M7.5 7.5h1M11.5 5h1" /></>,
  pr: <><circle cx="4" cy="3.5" r="1.75" /><circle cx="4" cy="12.5" r="1.75" /><circle cx="12" cy="12.5" r="1.75" /><path d="M4 5.25v5.5M12 10.75V6.25a1.5 1.5 0 0 0-1.5-1.5H8M9.75 3 8 4.75l1.75 1.75" /></>,
  issue: <><circle cx="8" cy="8" r="6.25" /><circle cx="8" cy="8" r="1.5" /></>,
  link: <path d="M6.75 9.25a3 3 0 0 0 4.25 0l2-2a3 3 0 0 0-4.25-4.25l-.75.75M9.25 6.75a3 3 0 0 0-4.25 0l-2 2A3 3 0 0 0 7.25 13l.75-.75" />,
  more: <path d="M3.5 8v.01M8 8v.01M12.5 8v.01" />,
  bolt: <path d="M8.75 1.75 3 9.25h4.5l-.75 5 5.75-7.5H8Z" />,
  shield: <path d="M8 1.75 2.75 3.75v3.9c0 3.1 2.2 5.6 5.25 6.6 3.05-1 5.25-3.5 5.25-6.6v-3.9Z" />,
  teach: <><path d="M1.75 5.5 8 2.5l6.25 3L8 8.5Z" /><path d="M4.25 7v3.25c0 .9 1.7 2 3.75 2s3.75-1.1 3.75-2V7M14.25 5.5v4" /></>,
  newChat: <><path d="M13.75 8.5c-.4 2.6-2.8 4.45-5.75 4.45-.8 0-1.55-.15-2.25-.42L2.5 13.5l.95-2.6A5.1 5.1 0 0 1 2.25 7.6c0-2.95 2.57-5.35 5.75-5.35" /><path d="M12 1.75v4.5M9.75 4h4.5" /></>,
  person: <><circle cx="8" cy="5.5" r="2.75" /><path d="M2.75 14c.5-2.6 2.6-4.25 5.25-4.25S12.75 11.4 13.25 14" /></>,
  bell: <><path d="M4 6.5a4 4 0 0 1 8 0c0 2.6.7 4 1.5 4.75H2.5C3.3 10.5 4 9.1 4 6.5Z" /><path d="M6.5 13.5a1.6 1.6 0 0 0 3 0" /></>,
  help: <><circle cx="8" cy="8" r="6.25" /><path d="M6.4 6.3a1.65 1.65 0 1 1 2.4 1.47c-.5.26-.8.62-.8 1.08v.35M8 11.25v.01" /></>,
  sidebar: <><rect x="1.75" y="2.25" width="12.5" height="11.5" rx="2" /><path d="M6 2.25v11.5" /></>,
  fit: <path d="M2.25 5.5V3.25c0-.55.45-1 1-1H5.5M10.5 2.25h2.25c.55 0 1 .45 1 1V5.5M13.75 10.5v2.25c0 .55-.45 1-1 1H10.5M5.5 13.75H3.25c-.55 0-1-.45-1-1V10.5" />,
  expand: <path d="M9.5 2.25h4.25V6.5M13.75 2.25 9.25 6.75M6.5 13.75H2.25V9.5M2.25 13.75l4.5-4.5" />,
  warn: <><path d="M7.13 2.5a1 1 0 0 1 1.74 0l5.5 9.75a1 1 0 0 1-.87 1.5H2.5a1 1 0 0 1-.87-1.5Z" /><path d="M8 6.25v3M8 11.5v.01" /></>,
  archive: <><rect x="1.75" y="2.75" width="12.5" height="3" rx="1" /><path d="M3 5.75v6.5a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1v-6.5M6.5 8.5h3" /></>,
  reply: <path d="M6.5 3.5 2.75 7.25 6.5 11M2.75 7.25h6.5a4 4 0 0 1 4 4v1.25" />,
  paperclip: <path d="m13.25 7.4-5.3 5.3a3.25 3.25 0 0 1-4.6-4.6l5.65-5.65a2.1 2.1 0 0 1 3 3L6.3 11.1a.95.95 0 0 1-1.35-1.35l5.1-5.1" />,
  image: <><rect x="1.75" y="2.75" width="12.5" height="10.5" rx="1.75" /><circle cx="5.5" cy="6.25" r="1.25" /><path d="m1.75 11.25 3.6-3.1a1 1 0 0 1 1.3 0l1.6 1.4 2.1-1.85a1 1 0 0 1 1.3 0l2.6 2.3" /></>,
} as const;

export type IconName = keyof typeof P;

export function Icon({ name, size = 16, className, strokeWidth = 1.5, title }: { name: IconName; size?: number; className?: string; strokeWidth?: number; title?: string }) {
  return (
    <svg
      width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={strokeWidth}
      strokeLinecap="round" strokeLinejoin="round" className={className} aria-hidden={title ? undefined : true} role={title ? "img" : undefined}
    >
      {title ? <title>{title}</title> : null}
      {P[name]}
    </svg>
  );
}

export const CHANNEL_ICON: Record<ChannelKind, IconName> = {
  teams: "chat", slack: "hash", email: "mail", jira: "ticket", asana: "checklist", azureDevOps: "board",
  github: "pr", linear: "issue", phone: "phone", other: "link",
};

/** The Omega Star mark: an omega cradling a four-point star. Tile and glyph follow the theme's primary colours. */
export function LogoMark({ size = 28, tile = true }: { size?: number; tile?: boolean }) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-label="Omega Star" role="img" className="logomark">
      {tile ? <rect width="32" height="32" rx="9" fill="var(--accent)" /> : null}
      <path d="M8.5 24.5h4v-2.6A7.6 7.6 0 1 1 19.5 21.9v2.6h4" stroke={tile ? "var(--accent-ink)" : "currentColor"} strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round" fill="none" />
      <path d="M16 9.6c.4 2.55 1.3 3.45 3.85 3.85-2.55.4-3.45 1.3-3.85 3.85-.4-2.55-1.3-3.45-3.85-3.85 2.55-.4 3.45-1.3 3.85-3.85Z" fill={tile ? "var(--accent-ink)" : "currentColor"} />
    </svg>
  );
}
