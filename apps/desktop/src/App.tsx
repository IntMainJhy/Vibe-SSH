import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { call, explain, type Host, type HostDraft, type Job, type Space } from "./api";
import { HostForm } from "./HostForm";
import { HostImport } from "./HostImport";
import { Forwards } from "./Forwards";
import { Icon } from "./Icon";
import { Modal } from "./Modal";
import { QuickSwitch } from "./QuickSwitch";
import { usePreferences } from "./preferences";
import { useWorkspace } from "./useWorkspace";
import { Files } from "./Files";
import { History } from "./History";
import { Workbench, STATUS, failedJobs, type HomeView } from "./Workbench";
import { Resources } from "./Resources";
import { TransferTray, useTransfers, type TransferStart } from "./Transfers";
import brandIcon from "../src-tauri/icons/128x128.png";

const Terminal = lazy(() =>
  import("./Terminal").then((module) => ({ default: module.Terminal })),
);
type Page = "terminal" | "files" | "resources";
type Notice = { text: string; kind: "error" | "success" };
type Act = (key: string, work: () => Promise<unknown>) => Promise<void>;
const homeTitles: Record<HomeView, string> = {
  overview: "服务器工作台",
  servers: "服务器",
  sessions: "当前会话",
  history: "任务历史",
};

export function App() {
  const { data, error, refresh } = useWorkspace();
  const preferences = usePreferences();
  const transfers = useTransfers();
  const [selected, select] = useState<string | null>(null);
  const [homeView, setHomeView] = useState<HomeView>("overview");
  const [views, setViews] = useState<
    Record<string, { page: Page; path: string }>
  >({});
  const [picked, setPicked] = useState<Record<string, string>>({});
  const [searching, setSearching] = useState(false);
  const [editing, setEditing] = useState<Host | "new">();
  const [importing, setImporting] = useState(false);
  const [draft, setDraft] = useState<HostDraft>();
  const [notice, setNotice] = useState<Notice>();
  const [quitting, setQuitting] = useState(false);
  const [pending, setPending] = useState<string[]>([]);
  const inflight = useRef(new Set<string>());
  const running =
    data?.spaces.flatMap((s) => s.jobs).filter((j) => j.status === "running")
      .length ?? 0;
  const runningRef = useRef(running);
  runningRef.current = running + transfers.active + (data?.open_documents ?? 0);
  const notify = useCallback(
    (text: string) => setNotice({ text, kind: "error" }),
    [],
  );

  const act: Act = async (key, work) => {
    if (inflight.current.has(key)) return;
    inflight.current.add(key);
    setPending([...inflight.current]);
    try {
      await work();
      await refresh();
    } catch (e) {
      notify(explain(e));
    } finally {
      inflight.current.delete(key);
      setPending([...inflight.current]);
    }
  };
  const copy = (text: string) =>
    act("copy", async () => {
      await call("copy_text", { text });
      setNotice({ text: "已复制到剪贴板", kind: "success" });
    });

  useEffect(() => {
    const stop = getCurrentWindow().onCloseRequested((event) => {
      if (runningRef.current > 0) {
        event.preventDefault();
        setQuitting(true);
      }
    });
    const quit = listen("quit-requested", () => setQuitting(true));
    const keydown = (event: KeyboardEvent) => {
      const commandKey = navigator.platform.startsWith("Mac")
        ? event.metaKey
        : event.ctrlKey;
      if (commandKey && !document.querySelector("dialog[open]")) {
        if (event.key.toLowerCase() === "k") {
          event.preventDefault();
          setSearching(true);
        }
        if (event.key.toLowerCase() === "n") {
          event.preventDefault();
          setEditing("new");
        }
      }
    };
    addEventListener("keydown", keydown);
    return () => {
      void stop.then((off) => off()).catch(() => {});
      void quit.then((off) => off()).catch(() => {});
      removeEventListener("keydown", keydown);
    };
  }, []);

  useEffect(() => {
    if (notice?.kind !== "success") return;
    const timer = setTimeout(() => setNotice(undefined), 3500);
    return () => clearTimeout(timer);
  }, [notice]);

  // A closed space returns to the workbench; it never silently selects another terminal.
  const space = data?.spaces.find((s) => s.space === selected);
  const sessionPage = (space && views[space.space]?.page) ?? "terminal";
  const destination = space ? "sessions" : homeView;
  const spaceId = space?.space;
  const viewKey = spaceId ? `${spaceId}:${sessionPage}` : homeView;
  const previousView = useRef({ key: viewKey, spaceId });
  const viewElement = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const previous = previousView.current;
    previousView.current = { key: viewKey, spaceId };
    if (previous.key === viewKey || matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const main = viewElement.current;
    // Within a session, keep the tab strip fixed while the selected content enters.
    const target = spaceId && previous.spaceId === spaceId
      ? main?.querySelector(".terminal-shell, .files-page, .resources-page")
      : main;
    const animation = target?.animate(
      [{ opacity: 0.78, transform: "translateY(5px)" }, { opacity: 1, transform: "none" }],
      { duration: 240, easing: "cubic-bezier(.22, .8, .24, 1)" },
    );
    return () => animation?.cancel();
  }, [viewKey, spaceId]);
  const hosts = data?.hosts ?? [];
  const sessions = data?.spaces ?? [];
  const goHome = (view: HomeView = "overview") => {
    select(null);
    setHomeView(view);
  };
  const goSpace = (id: string, page?: Page, job?: string) => {
    select(id);
    if (page)
      setViews((old) => ({
        ...old,
        [id]: { page, path: old[id]?.path ?? "." },
      }));
    if (job) setPicked((old) => ({ ...old, [id]: job }));
  };
  const openTerminal = (host: Host) =>
    act(`connect:${host.id}`, async () => {
      const opened = await call<{ space: string }>("open_terminal", {
        host: host.id,
      });
      goSpace(opened.space, "terminal");
    });

  const agents =
    data?.spaces.filter(
      (s) =>
        s.origin === "agent" &&
        s.connected &&
        s.control.owner === "agent" &&
        s.jobs.some((j) => j.status === "running"),
    ) ?? [];
  return (
    <div
      className="app workspace-layout"
      data-agent-active={agents.length > 0}
      data-session={!!space}
      data-view={space ? "session" : homeView}
    >
      <header className="workspace-toolbar" data-tauri-drag-region>
        {space ? (
          <label className="session-switcher" title="切换会话">
            <Icon name="terminal" size={16} />
            <span aria-hidden="true">切换会话</span>
            <Icon name="chevron" size={13} />
            <select aria-label="切换会话" value={space.space} onChange={e => goSpace(e.target.value)}>
              {sessions.map(s => (
                <option key={s.space} value={s.space}>
                  {s.origin === "agent" ? `${s.actor} · ${s.name}` : s.host_name}{!s.connected ? "（已断开）" : ""}
                </option>
              ))}
            </select>
          </label>
        ) : (
          <div className="workspace-toolbar-actions">
            {homeView === "servers" && (
              <button className="import-server" aria-label="导入配置" title="导入配置" onClick={() => setImporting(true)}>
                <Icon name="folder" size={15} />
                <span>导入配置</span>
              </button>
            )}
            <button className="add-server" aria-label="添加服务器" title="添加服务器（⌘N）" onClick={() => setEditing("new")}>
              <Icon name="plus" size={16} />
              <span>添加服务器</span>
            </button>
          </div>
        )}
      </header>
      <div className="workspace-rail">
        <nav className="workspace-navigation" aria-label="工作台页面">
          <button className="workspace-brand" aria-label="Vibe-SSH 总览" onClick={() => goHome()}>
            <img src={brandIcon} alt="" width={42} height={42} />
          </button>
          <div className="workspace-destinations" data-page={destination}>
            {([
              ["overview", "grid", "工作台"],
              ["servers", "server", "服务器"],
              ["sessions", "terminal", "全部会话"],
              ["history", "history", "任务历史"],
            ] as const).map(([view, icon, label]) => (
              <button key={view} aria-label={label} title={label}
                aria-current={!space && homeView === view ? "page" : undefined}
                data-selected={destination === view}
                onClick={() => goHome(view)}>
                <Icon name={icon} size={19} />
              </button>
            ))}
          </div>
        </nav>
        <div className="workspace-appearance">
          <label title="界面外观">
            <Icon name={preferences.theme === "dark" ? "moon" : "sun"} size={16} />
            <select aria-label="界面外观" value={preferences.theme} onChange={e => preferences.setTheme(e.target.value as typeof preferences.theme)}>
              <option value="system">跟随系统</option>
              <option value="light">浅色模式</option>
              <option value="dark">深色模式</option>
            </select>
          </label>
        </div>
      </div>
      <div className="workspace-surface">
        <aside className="workspace-explorer" aria-label="会话和服务器">
          <h2 className="workspace-explorer-title">工作空间</h2>
          <div className="workspace-explorer-list" tabIndex={0}>
            <Section title="我的会话" count={sessions.filter(s => s.origin === "human").length}>
              {sessions.filter(s => s.origin === "human").map(s => <button className="workspace-session" key={s.space}
                aria-current={s.space === space?.space ? "page" : undefined} onClick={() => goSpace(s.space)} title={s.host_name}>
                <Icon name="terminal" size={16} /><span>{s.host_name}<small>{s.connected ? "个人终端" : "连接已断开"}</small></span>
                <span className="dot" data-status={s.connected ? "succeeded" : "unknown"} />
              </button>)}
              {!sessions.some(s => s.origin === "human") && <p className="nav-empty">连接服务器，开始工作。</p>}
            </Section>
            {sessions.some(s => s.origin === "agent") && <Section title="Agent 空间" count={sessions.filter(s => s.origin === "agent").length}>
              {sessions.filter(s => s.origin === "agent").map(s => <button className="workspace-session" key={s.space}
                aria-current={s.space === space?.space ? "page" : undefined} onClick={() => goSpace(s.space)} title={`${s.host_name} · ${s.name}`}>
                <Icon name="link" size={16} /><span>{s.name}<small>{!s.connected ? "连接已断开" : s.control.note ? "等待你处理" : s.control.owner === "human" ? "你已接管" : `${s.actor} 操作中`}</small></span>
                {s.control.note || failedJobs(s) > 0 ? <Icon name="alert" size={14} /> : <span className="dot" data-status={s.jobs.some(j => j.status === "running") ? "running" : "succeeded"} />}
              </button>)}
            </Section>}
            <Section title="快速连接" count={hosts.length} action={<button className="icon-button" aria-label="添加服务器" title="添加服务器（⌘N）" onClick={() => setEditing("new")}><Icon name="plus" size={15} /></button>}>
              {hosts.map(host => <div className="workspace-host" key={host.id}>
                <button title={`新建连接 · ${host.username}@${host.host}:${host.port}`} disabled={pending.includes(`connect:${host.id}`)} onClick={() => void openTerminal(host)}>
                  <Icon name="server" size={15} /><span>{host.name}<small>{pending.includes(`connect:${host.id}`) ? "正在连接…" : host.host}</small></span>
                </button>
                <button className="icon-button" aria-label={`设置 ${host.name}`} onClick={() => setEditing(host)}><Icon name="settings" size={14} /></button>
              </div>)}
            </Section>
          </div>
          <div className="workspace-explorer-status" role="status">
            <span className="dot" data-status={error ? "unknown" : data ? "succeeded" : "running"} />
            {error ? "服务异常" : data ? "服务已就绪" : "连接中"}
          </div>
        </aside>
        <div className="workspace-detail">
          <main
            className={`main ${space ? "in-session" : ""}`}
            aria-label={space ? undefined : homeTitles[homeView]}
            ref={viewElement}
          >
            {data?.integration &&
            (data.integration.error || data.integration.skipped?.length) ? (
              <p className="integration-warning" role="status">
                接入配置需要处理：已有同名内容或目录不可写。请检查配置目录。
              </p>
            ) : null}
            {error && (
              <div className="error-banner" role="alert">
                <Icon name="alert" size={16} />
                <span>{error}</span>
                <button onClick={() => void refresh()}>重试</button>
              </div>
            )}
            {data?.status.history?.error && (
              <p className="form-error" role="alert">
                {data.status.history.error}
              </p>
            )}
            {!data ? (
              <div className="loading">
                <Icon name="terminal" size={32} />
                <p>{error ? "无法加载工作台" : "正在加载工作空间…"}</p>
                <button onClick={() => void refresh()}>重新加载</button>
              </div>
            ) : space ? (
              <Session
                key={space.space}
                space={space}
                host={data.hosts.find((h) => h.id === space.host)}
                job={
                  space.jobs.find((j) => j.job === picked[space.space]) ??
                  space.jobs.at(-1)
                }
                onPick={(job) =>
                  setPicked((previous) => ({ ...previous, [space.space]: job }))
                }
                fontSize={preferences.fontSize}
                setFontSize={preferences.setFontSize}
                act={act}
                pending={pending}
                onError={notify}
                copy={copy}
                onReconnect={openTerminal}
                page={sessionPage}
                onPage={(page) =>
                  setViews((old) => ({
                    ...old,
                    [space.space]: { path: old[space.space]?.path ?? ".", page },
                  }))
                }
                filePath={views[space.space]?.path ?? "."}
                onDirectory={(path) =>
                  setViews((old) => ({
                    ...old,
                    [space.space]: {
                      page: old[space.space]?.page ?? "files",
                      path,
                    },
                  }))
                }
                transfer={transfers.transfer}
                transferRevision={transfers.revision}
              />
            ) : homeView === "history" ? (
              <History revision={transfers.revision} />
            ) : (
              <Workbench
                data={data}
                hosts={hosts}
                query=""
                pending={pending}
                onConnect={openTerminal}
                onEdit={setEditing}
                sessions={sessions}
                view={homeView}
                onView={goHome}
                onSelect={goSpace}
              />
            )}
          </main>
          {agents.length > 0 && (
            <button
              className="agent-activity"
              aria-label={`${agents[0].actor} 正在操作 ${agents[0].name}，点击查看空间`}
              onClick={() => goSpace(agents[0].space)}
              title={agents.map((s) => `${s.actor} · ${s.name}`).join("\n")}
            >
              <span className="activity-orb" />
              <span>
                {agents[0].actor} 正在操作 · {agents[0].name}
                {agents.length > 1 ? ` +${agents.length - 1}` : ""}
              </span>
              <Icon name="arrow" size={14} />
            </button>
          )}
        </div>
      </div>
      <TransferTray state={transfers} spaces={data?.spaces ?? []} selected={selected} />
      {searching && <QuickSwitch hosts={hosts} spaces={sessions} onSelect={goSpace} onConnect={openTerminal} onClose={() => setSearching(false)} />}
      {importing && <HostImport onClose={() => setImporting(false)} onSelect={(host) => {
        setImporting(false);
        setDraft(host);
        setEditing("new");
      }} />}
      {editing && (
        <HostForm
          initial={editing === "new" ? undefined : editing}
          draft={draft}
          hosts={data?.hosts ?? []}
          onClose={() => {
            setEditing(undefined);
            setDraft(undefined);
            void refresh();
          }}
        />
      )}
      {quitting && (
        <Modal titleId="quit-title" onClose={() => setQuitting(false)}>
          <span className="modal-icon">
            <Icon name="alert" size={25} />
          </span>
          <h2 id="quit-title">退出 Vibe-SSH？</h2>
          <p>
            还有 {running + transfers.active} 个任务或传输在运行，
            {data?.open_documents ?? 0}{" "}
            份文本文件打开。退出会丢弃未保存编辑并断开所有连接，Agent
            也无法继续操作。远端程序是否停止需要另行确认。
          </p>
          <div className="actions">
            <button onClick={() => setQuitting(false)} autoFocus>
              继续运行
            </button>
            <button
              className="danger"
              onClick={() => void act("exit", () => call("exit_app"))}
            >
              断开并退出
            </button>
          </div>
        </Modal>
      )}
      {notice && (
        <div
          className="notice"
          data-kind={notice.kind}
          role={notice.kind === "error" ? "alert" : "status"}
        >
          <Icon name={notice.kind === "error" ? "alert" : "check"} size={18} />
          <span>{notice.text}</span>
          <button
            className="icon-button"
            aria-label="关闭提示"
            onClick={() => setNotice(undefined)}
          >
            <Icon name="close" size={16} />
          </button>
        </div>
      )}
    </div>
  );
}

function Section({
  title,
  count,
  action,
  children,
}: {
  title: string;
  count: number;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="nav-section">
      <div className="section-label">
        <h3>{title}</h3>
        <span>{count}</span>
        {action}
      </div>
      {children}
    </section>
  );
}

function Session({
  space,
  host,
  job,
  onPick,
  fontSize,
  setFontSize,
  act,
  pending,
  onError,
  copy,
  onReconnect,
  page,
  onPage,
  filePath,
  onDirectory,
  transfer,
  transferRevision,
}: {
  space: Space;
  host?: Host;
  job?: Job;
  onPick: (j: string) => void;
  fontSize: number;
  setFontSize: (size: number) => void;
  act: Act;
  pending: string[];
  onError: (e: string) => void;
  copy: (text: string) => Promise<void>;
  onReconnect: (h: Host) => Promise<void>;
  page: Page;
  onPage: (page: Page) => void;
  filePath: string;
  onDirectory: (path: string) => void;
  transfer: TransferStart;
  transferRevision: number;
}) {
  const [closing, setClosing] = useState(false);
  const [showForwards, setShowForwards] = useState(false);
  const held = space.control.owner === "human";
  const byAgent = space.origin === "agent";
  const accessDisabled = byAgent && !host?.agent_access;
  const id = space.space;
  const busy = pending.includes(`space:${id}`);
  const writable = held && space.connected && job?.status === "running";
  useEffect(() => {
    if (page === "terminal" && job)
      document
        .getElementById(`tab-${job.job}`)
        ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [job?.job, page]);
  return (
    <>
      <header className="session-heading">
        <div className="session-title">
          <span className="server-tile">
            <Icon name="terminal" size={22} />
          </span>
          <div>
            <h1>{byAgent ? space.name : space.host_name}</h1>
            <p>
              {host
                ? `${host.username}@${host.host}:${host.port}`
                : space.host_name}
              <span className={`badge ${held ? "human" : "agent"}`}>
                {byAgent ? (held ? "你已接管" : "Agent 操作中") : "我的终端"}
              </span>
            </p>
          </div>
        </div>
        <div className="actions">
          {!byAgent && space.connected && <button className="icon-button" aria-label="端口转发" title="端口转发" onClick={() => setShowForwards(true)}><Icon name="link" size={17} /></button>}
          {byAgent && space.connected && (!held || !accessDisabled) && (
            <button
              className={held ? "give" : "take"}
              disabled={busy}
              onClick={() =>
                void act(`space:${id}`, () =>
                  call(held ? "release" : "takeover", { space: id }),
                )
              }
            >
              <Icon name={held ? "arrow" : "terminal"} size={16} />
              {busy ? "处理中…" : held ? "交还给 Agent" : "接管会话"}
            </button>
          )}
          {held && space.connected && (
            <button
              disabled={busy}
              onClick={() =>
                void act(`space:${id}`, async () => {
                  onPick(await call<string>("shell", { space: id }));
                  onPage("terminal");
                })
              }
            >
              <Icon name="plus" size={16} />
              新终端
            </button>
          )}
          <button
            className="icon-button"
            title="关闭会话"
            aria-label="关闭会话"
            onClick={() => setClosing(true)}
          >
            <Icon name="close" />
          </button>
        </div>
      </header>
      {showForwards && space.connected && <Forwards space={id} onClose={() => setShowForwards(false)} />}
      {(accessDisabled || space.control.note) && (
        <div className="handoff" role="status">
          <Icon name="alert" size={18} />
          <div>
            <strong>{accessDisabled ? "Agent 接入已关闭" : "需要你处理"}</strong>
            <p>{accessDisabled ? "此会话由你控制，仍可查看输出或中断任务。" : space.control.note}</p>
          </div>
        </div>
      )}
      {!space.connected && (
        <div className="disconnect-banner">
          <Icon name="alert" size={17} />
          <span>连接已断开，远端执行结果需要核实。</span>
          {!byAgent && host && (
            <button
              disabled={pending.includes(`connect:${host.id}`)}
              onClick={() => void onReconnect(host)}
            >
              <Icon name="refresh" size={14} />
              重新连接
            </button>
          )}
        </div>
      )}
      <nav className="session-pages" aria-label="空间页面" data-page={page}>
        <button
          aria-current={page === "terminal" ? "page" : undefined}
          onClick={() => onPage("terminal")}
        >
          <Icon name="terminal" size={16} />
          终端
        </button>
        <button
          aria-current={page === "files" ? "page" : undefined}
          onClick={() => onPage("files")}
        >
          <Icon name="folder" size={16} />
          文件
        </button>
        <button
          aria-current={page === "resources" ? "page" : undefined}
          onClick={() => onPage("resources")}
        >
          <Icon name="activity" size={16} />
          监控
        </button>
      </nav>
      {page === "files" ? (
        <Files
          server={space.host_name}
          space={id}
          writable={held && space.connected}
          transfer={transfer}
          revision={transferRevision}
          initialPath={filePath}
          onDirectory={onDirectory}
        />
      ) : page === "resources" ? (
        <Resources space={id} />
      ) : (
        <div className="terminal-shell">
          <div
            className="terminal-tabs"
            role="tablist"
            aria-label="会话中的任务"
          >
            {space.jobs.map((j) => (
              <button
                key={j.job}
                role="tab"
                id={`tab-${j.job}`}
                aria-controls="terminal-panel"
                aria-selected={j.job === job?.job}
                tabIndex={j.job === job?.job ? 0 : -1}
                title={j.command ?? "交互终端"}
                onClick={() => onPick(j.job)}
                onKeyDown={(e) => {
                  if (
                    !["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)
                  )
                    return;
                  e.preventDefault();
                  const at = space.jobs.indexOf(j);
                  const index =
                    e.key === "Home"
                      ? 0
                      : e.key === "End"
                        ? space.jobs.length - 1
                        : (at +
                            (e.key === "ArrowLeft" ? -1 : 1) +
                            space.jobs.length) %
                          space.jobs.length;
                  onPick(space.jobs[index].job);
                  document
                    .getElementById(`tab-${space.jobs[index].job}`)
                    ?.focus();
                }}
              >
                <span className="dot" data-status={j.status} />
                <span>
                  {j.command
                    ? j.command.split("\n")[0].slice(0, 42)
                    : "交互终端"}
                </span>
                <span className="tab-kind" data-status={j.status}>
                  {STATUS[j.status]}
                </span>
              </button>
            ))}
            <span className="tab-spacer" />
            <span className="terminal-mode">
              {writable
                ? "可输入"
                : job?.status === "unknown"
                  ? "结果未知 · 只读"
                  : job && job.status !== "running"
                    ? "已结束 · 只读"
                    : "只读"}
            </span>
          </div>
          {job ? (
            <>
              <div
                id="terminal-panel"
                className="terminal-panel"
                role="tabpanel"
                aria-labelledby={`tab-${job.job}`}
              >
                <Suspense
                  fallback={<p className="terminal-loading">正在加载终端…</p>}
                >
                  <Terminal
                    key={job.job}
                    space={id}
                    job={job.job}
                    pty={job.pty}
                    writable={Boolean(writable)}
                    fontSize={fontSize}
                    onFontSize={setFontSize}
                    onCopyCommand={job.command ? () => void copy(job.command!) : undefined}
                    onError={onError}
                  />
                </Suspense>
              </div>
              <footer className="statusbar">
                <span>
                  <span className="dot" data-status={job.status} />
                  {STATUS[job.status]}
                  {job.exit_code !== null && (
                    <span className="exit-code">退出码 {job.exit_code}</span>
                  )}
                </span>
                <span>
                  {!space.connected
                    ? "已断开连接"
                    : job.status === "unknown"
                      ? held
                        ? "远端结果未知；点击“新终端”核实"
                        : "远端结果未知；接管后可新开终端核实"
                      : job.status !== "running"
                        ? held
                          ? "任务已结束；点击上方“新终端”继续操作"
                          : "任务输出只读；接管后可新开终端"
                        : !held
                          ? "只读观察 · 接管后可输入"
                          : "输入将发送到服务器"}
                  {held && job.status === "running" && space.connected && (
                    <button
                      className="interrupt"
                      disabled={busy}
                      onClick={() =>
                        void act(`space:${id}`, () =>
                          call("job_interrupt", { space: id, job: job.job }),
                        )
                      }
                    >
                      <Icon name="stop" size={12} />
                      中断
                    </button>
                  )}
                </span>
              </footer>
            </>
          ) : (
            <div className="terminal-empty">
              <Icon name="terminal" size={35} />
              <p>会话已连接，等待运行命令。</p>
              {held && (
                <button
                  disabled={busy}
                  onClick={() =>
                    void act(`space:${id}`, async () =>
                      onPick(await call<string>("shell", { space: id })),
                    )
                  }
                >
                  打开终端
                </button>
              )}
            </div>
          )}
        </div>
      )}
      {closing && (
        <Modal
          titleId="close-title"
          busy={busy}
          onClose={() => setClosing(false)}
        >
          <h2 id="close-title">关闭这个会话？</h2>
          <p>
            这会断开与 {space.host_name}{" "}
            的连接，并清除该会话的输出记录。已经启动的远端程序可能继续运行。
          </p>
          <div className="actions">
            <button disabled={busy} onClick={() => setClosing(false)} autoFocus>
              保留会话
            </button>
            <button
              className="danger"
              disabled={busy}
              onClick={() =>
                void act(`space:${id}`, () =>
                  call("close_space", { space: id }),
                )
              }
            >
              {busy ? "正在关闭…" : "关闭会话"}
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}
