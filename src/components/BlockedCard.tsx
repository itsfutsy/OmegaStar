import { useState } from "react";
import { call, errorMessage } from "../api/client";
import type { EmployeeView, Run } from "../api/types";
import { firstName } from "../lib/format";
import { refreshOrg } from "../lib/hooks";
import { toast } from "../lib/toasts";
import { FOLDER_TOOL, mcpServerName, mcpServerRule, toolName } from "../lib/rules";
import { stepWords } from "./Transcript";
import { Icon } from "./Icon";
import { ErrorNote } from "./ui";
import "./BlockedCard.css";

type Scope = "tool" | "server";

/** Whether the employee can already do this without asking: the rule itself, a server wildcard, or the browser switch. */
function covers(e: EmployeeView, rule: string): boolean {
  if (e.browser && rule.startsWith("mcp__claude-in-chrome__")) return true;
  return e.allowRules.some((r) => r === rule || (r.endsWith("__*") && rule.startsWith(r.slice(0, -1))));
}

/** Whether the folder is already one the employee works in, or inside one. */
function worksIn(e: EmployeeView, folder: string): boolean {
  return e.workspaces.some((w) => folder === w || folder.startsWith(w.endsWith("/") ? w : `${w}/`));
}

/** What a run was blocked on or asked for, and the way forward: allow the rules and folders once or for good, and
 *  the employee retries. */
export function BlockedCard({ run, employee }: { run: Run; employee: EmployeeView | undefined }) {
  const who = employee ? firstName(employee.name) : "They";
  const [scope, setScope] = useState<Record<number, Scope>>({});
  const [busy, setBusy] = useState<"once" | "always" | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const rows = run.denials.map((d, i) => {
    const server = mcpServerRule(d.tool);
    const sc: Scope = server ? scope[i] ?? "tool" : "tool";
    const rule = sc === "server" && server ? server : d.suggestedRule;
    const folderHeld = !!d.folder && !!employee && worksIn(employee, d.folder);
    return { d, i, server, sc, rule, already: !!rule && !!employee && covers(employee, rule), folderHeld };
  });
  const rules = [...new Set(rows.map((r) => r.rule).filter(Boolean))];
  const folders = [...new Set(rows.filter((r) => r.d.folder && !r.folderHeld).map((r) => r.d.folder as string))];
  const many = rows.length > 1;
  const askedN = rows.filter((r) => r.d.asked).length;
  const blockedN = rows.length - askedN;
  const blockedWords = blockedN === 1 ? "One action was blocked" : `${blockedN} actions were blocked`;
  const head = !askedN ? blockedWords
    : !blockedN ? `${who} asked for ${many ? `${rows.length} things` : "access"}`
    : `${blockedWords}, and ${who} asked for ${askedN} more`;
  const saves = [rules.length ? (rules.length === 1 ? "the rule" : "the rules") : null, folders.length ? (folders.length === 1 ? "the folder" : "the folders") : null].filter(Boolean).join(" and ");

  const approve = async (always: boolean) => {
    setBusy(always ? "always" : "once");
    setErr(null);
    try {
      await call("approve_blocked", { runId: run.id, rules, folders, always });
      if (always) refreshOrg();
      toast(always ? `${who} is retrying, and can do this from now on` : `${who} is retrying, with your OK for this run only`, "ok");
    } catch (e) {
      setErr(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="blocked" aria-label="Blocked actions">
      <div className="blocked-head"><Icon name="shield" size={15} /> {head}</div>
      <p className="blocked-lead">Allow {many ? "them" : "it"} and {who} picks the work back up where {many ? "they" : "it"} stopped.</p>
      <div className="blocked-rows">
        {rows.map((r) => {
          const server = mcpServerName(r.d.tool);
          return (
            <div key={r.i} className="blocked-row">
              <div className="blocked-what">
                <span className="blocked-tool">
                  <b className={r.d.tool === FOLDER_TOOL ? undefined : "mono"}>{r.d.tool === FOLDER_TOOL ? "Folder" : toolName(r.d.tool)}</b>
                  {server ? <span className="tag">{server}</span> : null}
                  {r.d.asked ? <span className="tag">Asked</span> : null}
                  {r.already ? <span className="tag tag-ok">Already allowed</span> : null}
                </span>
                {r.d.brief && r.d.asked ? <span className="blocked-brief"><span className="blocked-label">Why</span><span className="blocked-prose">{r.d.brief}</span></span> : null}
                {r.d.brief && !r.d.asked ? <span className="blocked-brief"><span className="blocked-label">Tried</span><span className={stepWords(r.d.tool, r.d.brief).code ? "mono" : "blocked-prose"}>{r.d.brief}</span></span> : null}
                {r.rule ? <span className="blocked-brief"><span className="blocked-label">Saves</span><span className="mono" title={r.rule}>{r.rule}</span></span> : null}
                {r.d.folder ? (
                  <span className="blocked-brief">
                    <span className="blocked-label">Folder</span><span className="mono" title={r.d.folder}>{r.d.folder}</span>
                    {r.folderHeld ? <span className="tag tag-ok">Already works there</span> : null}
                  </span>
                ) : null}
              </div>
              {r.server ? (
                <div className="seg blocked-scope" role="group" aria-label={`What to allow for ${toolName(r.d.tool)}`}>
                  <button type="button" aria-pressed={r.sc === "tool"} onClick={() => setScope((s) => ({ ...s, [r.i]: "tool" }))}>Just this tool</button>
                  <button type="button" aria-pressed={r.sc === "server"} onClick={() => setScope((s) => ({ ...s, [r.i]: "server" }))}>All {server} tools</button>
                </div>
              ) : null}
            </div>
          );
        })}
      </div>
      {err ? <ErrorNote text={err} /> : null}
      <div className="blocked-actions">
        <button className="btn btn-primary btn-sm" disabled={busy !== null} data-tip={`${who} retries, with your OK for this run only`} onClick={() => void approve(false)}>
          Allow once
        </button>
        <button className="btn btn-sm" disabled={busy !== null} data-tip={`Saves ${saves || "it"} to ${who}'s profile, and ${who} retries`} onClick={() => void approve(true)}>Always allow</button>
      </div>
    </section>
  );
}
