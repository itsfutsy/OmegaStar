import { useEffect, useState } from "react";
import { call, errorMessage } from "../api/client";
import { useOrg } from "../lib/hooks";
import { Icon } from "./Icon";
import "./UpdateCard.css";

type Update =
  | { kind: "none" }
  | { kind: "available"; version: string }
  | { kind: "installing"; version: string }
  | { kind: "failed"; version: string; message: string };

const DAY = 86_400_000;

export function UpdateCard() {
  const [update, setUpdate] = useState<Update>({ kind: "none" });
  const { snap } = useOrg();

  useEffect(() => {
    const showResult = (version: string | null) =>
      setUpdate((u) => (u.kind === "installing" || u.kind === "failed" ? u : version ? { kind: "available", version } : { kind: "none" }));
    const keepCardWhenOffline = () => {};
    const check = () => call("check_update").then(showResult, keepCardWhenOffline);
    void check();
    const timer = setInterval(check, DAY);
    return () => clearInterval(timer);
  }, []);

  if (update.kind === "none") return null;
  const { version } = update;
  const installing = update.kind === "installing";
  const working = (snap?.employees ?? []).some((e) => e.working > 0 || e.presence === "working");
  const install = () => {
    setUpdate({ kind: "installing", version });
    call("install_update").catch((e) => setUpdate({ kind: "failed", version, message: errorMessage(e) }));
  };

  return (
    <>
      <div className="update" aria-label="Update available">
        <div className="update-title">Update available</div>
        <div className="update-sub">Omega Star {version}</div>
        {working ? <div className="update-sub">Restarting stops work in progress.</div> : null}
        {update.kind === "failed" ? <div className="update-err">{update.message}</div> : null}
        <button className="btn btn-primary btn-sm update-btn" onClick={install} disabled={installing}>
          {installing ? "Updating…" : update.kind === "failed" ? "Try again" : "Restart to update"}
        </button>
      </div>
      <button className="update-mini" onClick={install} disabled={installing} data-tip={installing ? "Updating…" : `Update to ${version}`} aria-label={`Update Omega Star to ${version}`}>
        <Icon name="refresh" size={15} />
      </button>
    </>
  );
}
