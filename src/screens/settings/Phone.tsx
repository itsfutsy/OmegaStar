import { useEffect, useState, type ReactNode } from "react";
import { call, errorMessage } from "../../api/client";
import type { RemoteStatus } from "../../api/types";
import { useQuery } from "../../lib/hooks";
import { relTime } from "../../lib/format";
import { Icon } from "../../components/Icon";
import { Switch } from "../../components/ui";
import { Row, States, Swap, sentence, type Saves } from "./rows";
import { Group } from "./frame";

const PHONE = "Your phone";
const PUSH = "Notifications on your phone";

/** How long a code still works, as "4:59". */
function timeLeft(ms: number): string {
  const s = Math.max(0, Math.ceil(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Where a paired phone's link stands, in words. */
function linkWords(st: RemoteStatus): string {
  switch (st.state) {
    case "session": return "Connected now.";
    case "online": return st.lastSeenAt ? `Last connected ${relTime(st.lastSeenAt)}.` : "Waiting for it to connect.";
    case "pairing": return "Pairing a new phone.";
    case "off": case "connecting": return "Connecting…";
    default: {
      const _exhaustive: never = st.state;
      return _exhaustive;
    }
  }
}

/** The phone: Connect phone, then the code to scan for five minutes, then the paired phone with its notifications
 *  switch and Unpair. The code comes from the status, so it shows again when you come back while pairing. */
export function PhoneGroup({ saves }: { saves: Saves }) {
  const status = useQuery(() => call("remote_status"), [], ["remote:updated"]);
  const [now, setNow] = useState(() => Date.now());
  const [busy, setBusy] = useState(false);
  const st = status.data;
  const code = st?.code ?? null;
  const ticking = !!code;

  useEffect(() => {
    if (!ticking) return;
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [ticking]);

  const connect = async () => {
    saves.clear(PHONE);
    setBusy(true);
    try {
      await call("remote_pair_start");
      setNow(Date.now());
      status.reload();
    } catch (e) {
      saves.failed(PHONE, sentence(errorMessage(e)), () => void connect());
    } finally {
      setBusy(false);
    }
  };
  const cancel = async () => {
    try { await call("remote_pair_cancel"); status.reload(); }
    catch (e) { saves.failed(PHONE, sentence(errorMessage(e)), () => void cancel()); }
  };
  const unpair = async () => {
    saves.clear(PHONE);
    setBusy(true);
    try {
      await call("remote_unpair");
      status.reload();
    } catch (e) {
      saves.failed(PHONE, sentence(errorMessage(e)), () => void unpair());
    } finally {
      setBusy(false);
    }
  };
  const setPush = async (on: boolean) => {
    if (st) status.set({ ...st, pushOn: on });
    saves.saving(PUSH);
    try {
      status.set(await call("remote_set_push", { on }));
      saves.saved(PUSH);
    } catch (e) {
      status.reload();
      saves.failed(PUSH, sentence(errorMessage(e)), () => void setPush(on));
    }
  };

  const left = code ? Date.parse(code.expiresAt) - now : 0;
  const shown = code && left > 0 ? code : null;
  const paired = !!st?.deviceName;
  let desc: ReactNode;
  let control: ReactNode = null;
  if (!st) {
    desc = status.error ? <span className="set-err">{sentence(status.error)}</span> : "Checking…";
  } else if (shown) {
    desc = (
      <>
        Scan this with the OmegaStar app on your phone. It works for <span className="num">{timeLeft(left)}</span>.
        {st.error ? <span className="set-note set-err">{sentence(st.error)}</span> : null}
        <img className="set-qr" src={shown.qr} width={160} height={160} alt="Pairing code for the OmegaStar app on your phone." />
      </>
    );
    control = <button className="btn" onClick={() => void cancel()}>Cancel</button>;
  } else if (paired) {
    desc = (
      <>
        Paired with {st.deviceName}. {linkWords(st)}
        {st.error ? <span className="set-note set-err">{sentence(st.error)}</span> : null}
      </>
    );
    control = <button className="btn" onClick={() => void unpair()} disabled={busy}><States at={busy ? 1 : 0} labels={["Unpair", "Unpairing…"]} /></button>;
  } else {
    desc = "No phone is paired.";
    control = (
      <button className="btn" onClick={() => void connect()} disabled={busy}>
        <States at={busy ? 1 : 0} labels={[<><Icon name="phone" size={13} />Connect phone</>, "Starting…"]} />
      </button>
    );
  }
  const key = !st ? (status.error ? "error" : "wait") : shown ? `code${st.error}` : paired ? `paired${st.state}${st.error}` : "off";
  return (
    <Group title="Phone" sub="Read and answer your Inbox from the OmegaStar app on your phone. It reaches this computer through OmegaStar's relay, sealed so the relay can't read it.">
      <Row title={PHONE} desc={<Swap on={key}>{desc}</Swap>}>{control}</Row>
      {st && paired && !shown ? (
        <Row title={PUSH} desc="Your phone gets the notifications this computer shows. Their titles pass through the relay to reach it.">
          <Switch checked={st.pushOn} onChange={(v) => void setPush(v)} label="Notifications on your phone" />
        </Row>
      ) : null}
    </Group>
  );
}
