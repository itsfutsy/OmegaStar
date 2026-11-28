import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { ProxySettings } from "../../api/types";
import { ms, reducedMotion } from "../../lib/motion";
import { FailLine, FieldError, Row, SaveButton, Swap, useRowSave } from "./rows";
import { Group } from "./frame";

type ProxyErrors = Partial<Record<"url" | "managementKey", string>>;

/** Sends every run through a CLI proxy (CLIProxyAPI) instead of the signed-in account. While it is off the section is
 *  one row and Set up; its fields open in place. `confirmed` is what the app last saved, so a failed save keeps what
 *  you typed. */
export function ProxyGroup({ confirmed, onEdit, onSave }: {
  confirmed: ProxySettings; onEdit: () => void; onSave: (proxy: ProxySettings, then: () => void) => void;
}) {
  const confirmedKey = JSON.stringify(confirmed);
  const [draft, setDraft] = useState(confirmed);
  useEffect(() => { setDraft(JSON.parse(confirmedKey) as ProxySettings); }, [confirmedKey]);
  const [setup, setSetup] = useState(false);
  const [errs, setErrs] = useState<ProxyErrors>({});
  const save = useRowSave("CLI proxy");
  const address = useRef<HTMLInputElement>(null);
  const form = useRef<HTMLFormElement>(null);
  const ids = useId();
  const on = !!confirmed.url;
  const open = on || setup;
  const failed = save?.state === "failed";

  // Once the form has opened (or an error has opened under a field), scroll so its Save sits a section's gap above the
  // bottom of the page, without pushing its first row under the header.
  const reveal = useCallback(() => {
    const scroller = form.current?.closest<HTMLElement>(".pagecol-scroll");
    const foot = form.current?.querySelector<HTMLElement>(".set-foot");
    const first = form.current?.parentElement?.previousElementSibling;
    if (!scroller || !foot || !first) return;
    const view = scroller.getBoundingClientRect();
    const short = 48 - (view.bottom - foot.getBoundingClientRect().bottom);
    const room = first.getBoundingClientRect().top - view.top - 24;
    const by = Math.min(short, room);
    if (by > 0) scroller.scrollBy({ top: by, behavior: reducedMotion() ? "auto" : "smooth" });
  }, []);
  const afterOpening = useCallback(() => window.setTimeout(reveal, ms("--d-arrive") + 40), [reveal]);
  useEffect(() => {
    if (!setup) return;
    address.current?.focus({ preventScroll: true });
    const t = afterOpening();
    return () => window.clearTimeout(t);
  }, [setup, afterOpening]);
  useEffect(() => {
    if (!errs.url && !errs.managementKey && !failed) return;
    const t = afterOpening();
    return () => window.clearTimeout(t);
  }, [errs, failed, afterOpening]);

  const changed = (["url", "apiKey", "managementKey"] as const).some((k) => draft[k].trim() !== confirmed[k]);
  const edit = (key: keyof ProxySettings) => (e: { target: { value: string } }) => {
    setDraft({ ...draft, [key]: e.target.value });
    if (key in errs) setErrs({ ...errs, [key]: undefined });
    if (failed) onEdit();
  };
  const submit = () => {
    const next = { url: draft.url.trim().replace(/\/+$/, ""), apiKey: draft.apiKey.trim(), managementKey: draft.managementKey.trim() };
    const found: ProxyErrors = {};
    if (next.url && !/^https?:\/\//.test(next.url)) found.url = "The CLI proxy address must start with http:// or https://.";
    if (next.managementKey && next.managementKey === next.apiKey) found.managementKey = "The management key is the proxy's management secret, not one of its API keys.";
    setErrs(found);
    if (found.url) return address.current?.focus();
    if (found.managementKey) return;
    onSave(next, () => { if (!next.url) setSetup(false); });
  };
  const cancel = () => {
    setDraft(confirmed);
    setErrs({});
    setSetup(false);
    onEdit();
  };
  const field = (key: "url" | "apiKey" | "managementKey", label: string, extra: { placeholder?: string; type?: string } = {}) => {
    const err = key === "apiKey" ? undefined : errs[key];
    return (
      <div className="set-field">
        <input
          ref={key === "url" ? address : undefined} className="input mono set-input" value={draft[key]} onChange={edit(key)} aria-label={label}
          aria-invalid={err ? true : undefined} aria-describedby={err ? `${ids}${key}` : undefined} {...extra}
        />
        <FieldError id={`${ids}${key}`} text={err ?? null} />
      </div>
    );
  };
  return (
    <Group title="CLI proxy" sub="Send every run through a CLIProxyAPI server, so employees share its pool of Claude accounts.">
      <Row
        title="Status"
        desc={<Swap on={confirmed.url}>{on ? `Runs go through ${confirmed.url}. Claude in Chrome and claude.ai connectors still use Claude Code's own sign-in.` : "Off. Runs use the account Claude Code is signed in to."}</Swap>}
      >
        {open ? null : <button type="button" className="btn" onClick={() => setSetup(true)}>Set up</button>}
      </Row>
      <div className="collapse" data-open={open} inert={!open}>
        <form ref={form} onSubmit={(e) => { e.preventDefault(); submit(); }} noValidate>
          <Row title="Address" desc={on ? "Leave empty to turn the proxy off." : undefined} mark={false} fail={false}>
            {field("url", "CLI proxy address", { placeholder: "http://127.0.0.1:8317" })}
          </Row>
          <Row title="API key" desc="One of the keys in the proxy's config." mark={false} fail={false}>
            {field("apiKey", "CLI proxy API key", { type: "password" })}
          </Row>
          <Row title="Management key" desc={<>Not an API key: the <span className="mono">secret-key</span> under <span className="mono">management</span> in the proxy's config. The sidebar needs it to show the proxy's Claude usage limits.</>} mark={false} fail={false}>
            {field("managementKey", "CLI proxy management key", { type: "password" })}
          </Row>
          {/* A failed save opens the same line, with Try again, as on every other row. */}
          <div className="set-row set-foot">
            <div className="set-words">
              <FailLine id={`${ids}save`} save={save} />
            </div>
            <div className="set-control">
              {on ? null : <button type="button" className="btn btn-ghost" onClick={cancel}>Cancel</button>}
              <SaveButton state={failed ? undefined : save?.state} disabled={!changed} />
            </div>
          </div>
        </form>
      </div>
    </Group>
  );
}
