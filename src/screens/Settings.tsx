import { useEffect, useRef, type ReactNode } from "react";
import { call, errorMessage } from "../api/client";
import { rescanSkills, useQuery, useSkillScan } from "../lib/hooks";
import { capitalize, plural } from "../lib/format";
import { setTheme, useTheme, type ThemePref } from "../lib/theme";
import { CopyCommand } from "../components/CopyCommand";
import { Icon } from "../components/Icon";
import { ErrorNote, PageColumn, Switch } from "../components/ui";
import { SHELL, signInCommand } from "../lib/claudeCode";
import { openHelp } from "../lib/help";
import { Row, SaveRows, States, Stepper, Swap, sentence, useRowSaves, useSettingsSaves } from "./settings/rows";
import { Group, SectionIndex, SettingsLoading } from "./settings/frame";
import { PathRow, SkillsDesc } from "./settings/ClaudeCode";
import { CHANNELS, SupportCard, UpdateRow, aboutDesc } from "./settings/About";
import { NotificationsGroup, RepliesGroup } from "./settings/Notifications";
import { BackupsGroup } from "./settings/Backups";
import { PhoneGroup } from "./settings/Phone";
import { ProxyGroup } from "./settings/Proxy";
import "./Settings.css";

export function SettingsScreen() {
  const status = useQuery(() => call("setup_status"), []);
  const settings = useQuery(() => call("settings"), []);
  const theme = useTheme();
  const scan = useSkillScan();
  useEffect(() => { rescanSkills(); }, []);
  const saves = useRowSaves();
  const { view: s, change } = useSettingsSaves(settings, saves);
  const page = useRef<HTMLDivElement>(null);

  const setBackground = async (enabled: boolean) => {
    const row = "Keep OmegaStar running in the background";
    const before = status.data;
    if (before) status.set({ ...before, background: enabled });
    saves.saving(row);
    try {
      status.set(await call("set_background", { enabled }));
      saves.saved(row);
    } catch (e) {
      if (before) status.set(before);
      saves.failed(row, sentence(errorMessage(e)), () => void setBackground(enabled));
    }
  };
  const openData = async () => {
    saves.clear("Office folder");
    try { await call("open_data_dir"); }
    catch (e) { saves.failed("Office folder", sentence(errorMessage(e)), () => void openData()); }
  };

  const st = status.data;
  const err = (!settings.data && !settings.loading && settings.error) || (!st && !status.loading && status.error) || null;
  let body: ReactNode;
  if (err) {
    body = (
      <div className="set-state">
        <ErrorNote title="Couldn't load your settings" text={`${sentence(err)} Nothing was changed.`} onRetry={() => { settings.reload(); status.reload(); }} />
      </div>
    );
  } else if (!s || !st || !settings.data) {
    body = <SettingsLoading />;
  } else {
    const n = s.notifications;
    body = (
      <SaveRows.Provider value={saves.rows}>
        <div ref={page} className="set-page">
          <div className="set-main">
            <Group
              title="Claude Code"
              action={<button type="button" className="set-link" onClick={() => void openHelp()}>Claude Code docs<Icon name="external" size={12} /></button>}
            >
              <Row title="Installation" desc={<Swap on={`${st.claude.found}${st.claude.path}`}>{st.claude.found ? <span className="mono">{st.claude.path}</span> : "Not found. Install Claude Code or point to it below."}</Swap>}>
                {st.claude.found ? <span className="pill pill-ok"><Icon name="check" size={11} /> v{st.claude.version}</span> : <span className="pill pill-err">Not found</span>}
              </Row>
              <PathRow
                confirmed={settings.data.claudePath ?? ""}
                onEdit={() => saves.clear("Path override")}
                onSave={(path) => change("Path override", (x) => ({ ...x, claudePath: path || null }), () => status.reload())}
              />
              {s.proxy.url ? (
                <Row title="Account" desc="Runs use the CLI proxy's accounts, not the one Claude Code is signed in to.">
                  <span className="tag">CLI proxy</span>
                </Row>
              ) : (
                <Row title="Account" desc={st.account?.loggedIn ? <>{st.account.email} · {st.account.orgName}{st.account.plan ? ` · ${st.account.plan}` : null}</> : `Not signed in. Run this in ${SHELL}, then come back.`}>
                  {st.account?.loggedIn ? <span className="pill pill-ok"><Icon name="check" size={11} /> Signed in</span> : <CopyCommand command={signInCommand(st.claude.path)} />}
                </Row>
              )}
              <Row title="Skills" desc={<SkillsDesc scan={scan} />}>
                {scan.sources ? <span className={`set-count num ${scan.sources.found === 0 ? "set-count-warn" : ""}`}>{plural(scan.sources.found, "skill")}</span> : null}
                <button className="btn" onClick={rescanSkills} disabled={scan.scanning}>
                  <States at={scan.scanning ? 1 : 0} labels={[<><Icon name="refresh" size={13} />Rescan</>, "Scanning…"]} />
                </button>
              </Row>
            </Group>

            <Group title="Running">
              <Row title="Keep OmegaStar running in the background" desc="Schedules only run while OmegaStar is open. With this on, it starts at login and stays in the tray.">
                <Switch checked={st.background} onChange={(v) => void setBackground(v)} label="Run in the background" />
              </Row>
              <Row title="Pause all schedules" desc={<Swap on={String(s.paused)}>{s.paused ? "Nobody runs on a schedule until you resume. Manual runs and conversations still work." : "Stop every schedule at once, for a holiday or a noisy week."}</Swap>}>
                <Switch checked={s.paused} onChange={(v) => change("Pause all schedules", (x) => ({ ...x, paused: v }))} label="Pause schedules" />
              </Row>
              <Row title="Runs at the same time" desc="Across the office, and for any one employee. Extra work waits its turn. What you type to an employee is never held up; card answers and approvals wait like other work.">
                <Stepper
                  label="Runs at the same time" value={s.maxConcurrentRuns} min={1} max={16} less="Fewer runs at the same time" more="More runs at the same time"
                  onChange={(v) => change("Runs at the same time", (x) => ({ ...x, maxConcurrentRuns: v }))}
                />
              </Row>
            </Group>

            <NotificationsGroup n={n} change={change} />

            <RepliesGroup w={s.watch} change={change} />

            <PhoneGroup saves={saves} />

            <Group title="Appearance">
              <Row title="Theme" desc="System follows your computer's light or dark setting.">
                <div className="seg" role="group" aria-label="Theme">
                  {(["system", "light", "dark"] as ThemePref[]).map((t) => (
                    <button key={t} aria-pressed={theme === t} onClick={() => setTheme(t)}>
                      <Icon name={t === "system" ? "monitor" : t === "light" ? "sun" : "moon"} size={14} /> {capitalize(t)}
                    </button>
                  ))}
                </div>
              </Row>
            </Group>

            <BackupsGroup s={s} change={change} saves={saves} onOpenData={() => void openData()} />

            <ProxyGroup
              confirmed={settings.data.proxy}
              onEdit={() => saves.clear("CLI proxy")}
              onSave={(proxy, then) => change("CLI proxy", (x) => ({ ...x, proxy }), () => { status.reload(); then(); })}
            />

            <Group title="About" foot={<SupportCard />}>
              <Row title="OmegaStar" desc={aboutDesc(st)}>
                <span className="set-version num">v{st.build.version}</span>
                <span className="tag">{CHANNELS[st.build.channel]}</span>
                {st.sandbox && <span className="tag" title="This copy never starts Claude Code, backs up or watches anyone.">Sandbox</span>}
              </Row>
              <UpdateRow channel={st.build.channel} />
            </Group>
          </div>
          <SectionIndex host={page} />
          <p className="set-sr" aria-live="polite">{saves.said}</p>
        </div>
      </SaveRows.Provider>
    );
  }
  return <PageColumn title="Settings" className="set-col">{body}</PageColumn>;
}
