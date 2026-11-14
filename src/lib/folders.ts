// Workspace folders as the profile, hire sheet and hire proposals show them: each one checked by the office
// (`folders::check` in the backend), with a plain line saying whether it is there.

import { useEffect, useState } from "react";
import { call } from "../api/client";
import type { FolderCheck } from "../api/types";

export type FolderNote = { tone: "ok" | "err"; text: string };

/** What to say under a typed folder: found, missing, or why it can't be used. `null` while it is blank or being checked. */
export function folderNote(raw: string, check: FolderCheck | undefined): FolderNote | null {
  if (!raw.trim() || !check || check.raw !== raw) return null;
  if (check.problem) return { tone: "err", text: `This ${check.problem}.` };
  const where = check.path && check.path !== raw.trim() ? ` at ${check.path}` : "";
  return check.found ? { tone: "ok", text: `Found${where}` } : { tone: "err", text: `Not found${where}. Their runs won't start until it's there.` };
}

/** The office's check of each folder, by what was typed, asked again a moment after typing stops. */
export function useFolderChecks(paths: string[]): Map<string, FolderCheck> {
  const [checks, setChecks] = useState<Map<string, FolderCheck>>(new Map());
  const key = paths.filter((p) => p.trim()).join("\n");
  useEffect(() => {
    if (!key) return;
    let live = true;
    const t = window.setTimeout(() => {
      call("check_folders", { paths: key.split("\n") })
        .then((list) => { if (live) setChecks(new Map(list.map((c) => [c.raw, c]))); })
        .catch(() => { /* No line under the folder: saving still checks it. */ });
    }, 250);
    return () => { live = false; window.clearTimeout(t); };
  }, [key]);
  return checks;
}
