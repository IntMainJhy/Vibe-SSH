import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { call, type Host, type Job, type Snapshot, type Space } from "./api";
import { explain } from "./api";
import { HostForm } from "./HostForm";
import { Icon } from "./Icon";
import { Modal } from "./Modal";
import { usePreferences } from "./preferences";
import { useWorkspace } from "./useWorkspace";
import { Files } from "./Files";
import { Resources } from "./Resources";
import { TransferTray, useTransfers, type TransferStart } from "./Transfers";

const Terminal = lazy(() =>
  import("./Terminal").then((module) => ({ default: module.Terminal })),
);
const STATUS: Record<Job["status"], string> = {
  running: "运行中",
  succeeded: "已完成",
  failed: "失败",
  unknown: "结果未知",
};
const failedJobs = (space: Space) =>
  space.jobs.filter((job) => job.status === "failed").length;
type Page = "terminal" | "files" | "resources";
type Notice = { text: string; kind: "error" | "success" };
type Act = (key: string, work: () => Promise<unknown>) => Promise<void>;

export function App() {
  const { data, error, refreshing, refresh } = useWorkspace();
  const preferences = usePreferences();
  const transfers = useTransfers();
  const [selected, select] = useState<string | null>(null);
  const [views, setViews] = useState<
    Record<string, { page: Page; path: string }>
  >({});
  const [picked, setPicked] = useState<Record<string, string>>({});
  const [query, setQuery] = useState("");
  const search = useRef<HTMLInputElement>(null);
  const [editing, setEditing] = useState<Host | "new">();
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
          search.current?.focus();
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
  const needle = query.trim().toLocaleLowerCase();
  const matches = (...text: string[]) =>
    text.join(" ").toLocaleLowerCase().includes(needle);
  const hosts =
    data?.hosts.filter((h) => matches(h.name, h.host, h.username)) ?? [];
  const sessions =
    data?.spaces.filter((s) => matches(s.name, s.host_name, s.actor)) ?? [];
  const openTerminal = (host: Host) =>
    act(`connect:${host.id}`, async () => {
      const opened = await call<{ space: string }>("open_terminal", {
        host: host.id,
      });
      select(opened.space);
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
    <div className="app" data-agent-active={agents.length > 0}>
      <aside className="sidebar">
        <div className="traffic-space" data-tauri-drag-region />
        <div className="brand">
          <span className="brand-mark">
            <Icon name="terminal" size={22} />
          </span>
          <strong>
            Vibe<span>SSH</span>
          </strong>
          <span className="version" title={data?.status.version}>
            {data?.status.version.split("-")[0] ?? "0.2.0"}
          </span>
        </div>
        <label className="search">
          <Icon name="search" size={16} />
          <input
            ref={search}
            aria-label="搜索服务器或会话"
            placeholder="搜索服务器或会话"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                setQuery("");
                search.current?.blur();
              }
            }}
          />
          <kbd>⌘ K</kbd>
        </label>
        <button
          className="nav-home"
          data-active={!space}
          onClick={() => {
            select(null);
            setQuery("");
          }}
        >
          <Icon name="grid" />
          工作台<span>{data?.hosts.length ?? 0}</span>
        </button>
        <nav aria-label="服务器和会话">
          <Section
            title="服务器"
            count={hosts.length}
            action={
              <button
                className="icon-button"
                aria-label="添加服务器"
                title="添加服务器（⌘N）"
                onClick={() => setEditing("new")}
              >
                <Icon name="plus" size={16} />
              </button>
            }
          >
            {hosts.map((host) => (
              <div className="host-nav" key={host.id}>
                <button
                  className="host-nav-main"
                  title={`${host.username}@${host.host}:${host.port}`}
                  disabled={pending.includes(`connect:${host.id}`)}
                  onClick={() => void openTerminal(host)}
                >
                  <span className="server-glyph">
                    <Icon name="server" size={17} />
                  </span>
                  <span className="nav-text">
                    <strong>{host.name}</strong>
                    <small>
                      {pending.includes(`connect:${host.id}`)
                        ? "正在连接…"
                        : host.host}
                    </small>
                  </span>
                </button>
                <button
                  className="icon-button settings"
                  aria-label={`设置 ${host.name}`}
                  onClick={() => setEditing(host)}
                >
                  <Icon name="settings" size={15} />
                </button>
              </div>
            ))}
            {!hosts.length && (
              <p className="nav-empty">
                {query ? "没有匹配的服务器" : "还没有添加服务器"}
              </p>
            )}
          </Section>
          <Section title="会话" count={sessions.length}>
            {sessions.map((s) => (
              <button
                className="session-nav"
                key={s.space}
                data-active={s.space === space?.space}
                onClick={() => select(s.space)}
              >
                <span className="session-icon" data-kind={s.origin}>
                  <Icon
                    name={s.origin === "agent" ? "link" : "terminal"}
                    size={17}
                  />
                </span>
                <span className="nav-text">
                  <strong>{s.origin === "agent" ? s.name : s.host_name}</strong>
                  <small>
                    {!s.connected
                      ? "连接已断开"
                      : s.control.note
                        ? "等待你处理"
                        : s.origin === "agent"
                          ? s.control.owner === "human"
                            ? "你已接管"
                            : "Agent 操作"
                          : "我的终端"}
                    {failedJobs(s) > 0 && (
                      <span className="failure-note">
                        {" "}
                        · {failedJobs(s)} 项失败
                      </span>
                    )}
                  </small>
                </span>
                <span
                  className="dot"
                  data-status={
                    !s.connected
                      ? "unknown"
                      : s.jobs.some((j) => j.status === "running")
                        ? "running"
                        : "succeeded"
                  }
                />
              </button>
            ))}
            {!sessions.length && (
              <p className="nav-empty">
                {query ? "没有匹配的会话" : "连接服务器后，会话出现在这里"}
              </p>
            )}
          </Section>
        </nav>
        <footer className="sidebar-footer">
          {data?.integration &&
          (data.integration.error || data.integration.skipped?.length) ? (
            <p
              className="integration-warning"
              role="status"
              title={
                data.integration.error ||
                data.integration.skipped
                  ?.map((s) => `${s.path}: ${s.reason}`)
                  .join("\n")
              }
            >
              接入配置需要处理：已有同名内容或目录不可写。请检查配置目录。
            </p>
          ) : null}
          <label className="appearance">
            <Icon
              name={preferences.theme === "dark" ? "moon" : "sun"}
              size={15}
            />
            <select
              aria-label="界面外观"
              value={preferences.theme}
              onChange={(e) =>
                preferences.setTheme(e.target.value as typeof preferences.theme)
              }
            >
              <option value="system">跟随系统</option>
              <option value="light">浅色模式</option>
              <option value="dark">深色模式</option>
            </select>
            <span className="local-status">
              <span
                className="dot"
                data-status={error ? "unknown" : data ? "succeeded" : "running"}
              />
              {error ? "服务异常" : data ? "已就绪" : "连接中"}
            </span>
          </label>
        </footer>
      </aside>
      <main className={`main ${space ? "in-session" : ""}`}>
        <div className="window-bar" data-tauri-drag-region>
          <div className="breadcrumbs">
            <Icon name={space ? "terminal" : "grid"} size={15} />
            <button onClick={() => select(null)}>工作台</button>
            {space && (
              <>
                <span>/</span>
                <span>{space.host_name}</span>
              </>
            )}
          </div>
          <div className="window-meta">
            <span className="dot" data-status="succeeded" />
            SSH 工作空间
            <button
              className="icon-button"
              aria-label="刷新状态"
              disabled={refreshing}
              onClick={() => void refresh()}
            >
              <Icon name="refresh" size={15} />
            </button>
          </div>
        </div>
        {error && (
          <div className="error-banner" role="alert">
            <Icon name="alert" size={16} />
            <span>{error}</span>
            <button onClick={() => void refresh()}>重试</button>
          </div>
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
            page={views[space.space]?.page ?? "terminal"}
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
        ) : (
          <Workbench
            data={data}
            hosts={hosts}
            query={query}
            pending={pending}
            onConnect={openTerminal}
            onEdit={setEditing}
            onSelect={select}
          />
        )}
      </main>
      {agents.length > 0 && (
        <button
          className="agent-activity"
          aria-label={`${agents[0].actor} 正在操作 ${agents[0].name}，点击查看空间`}
          onClick={() => select(agents[0].space)}
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
      <TransferTray state={transfers} />
      {editing && (
        <HostForm
          initial={editing === "new" ? undefined : editing}
          onClose={() => {
            setEditing(undefined);
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
        <h2>{title}</h2>
        <span>{count}</span>
        {action}
      </div>
      {children}
    </section>
  );
}

function Workbench({
  data,
  hosts,
  query,
  pending,
  onConnect,
  onEdit,
  onSelect,
}: {
  data: Snapshot;
  hosts: Host[];
  query: string;
  pending: string[];
  onConnect: (h: Host) => Promise<void>;
  onEdit: (h: Host | "new") => void;
  onSelect: (s: string) => void;
}) {
  const running = data.spaces
    .flatMap((s) => s.jobs)
    .filter((j) => j.status === "running").length;
  const attention = data.spaces.filter((s) => s.control.note);
  return (
    <div className="workbench">
      <header className="page-heading">
        <div>
          <h1>服务器工作台</h1>
          <p>终端、文件与资源监控。</p>
        </div>
        <button
          className={data.hosts.length ? "add-server" : "primary"}
          onClick={() => onEdit("new")}
        >
          <Icon name="plus" size={17} />
          添加服务器
        </button>
      </header>
      <div className="workspace-summary">
        <span>
          <span className="dot" data-status="succeeded" />
          <strong>{data.hosts.length}</strong>台服务器
        </span>
        <span>
          <Icon name="terminal" size={16} />
          <strong>{data.spaces.length}</strong>个会话
        </span>
        <span>
          <Icon name="clock" size={16} />
          <strong>{running}</strong>个运行中任务
        </span>
        {attention.length > 0 && (
          <button
            onClick={() => onSelect(attention[0].space)}
            className="attention-link"
          >
            <Icon name="alert" size={15} />
            {attention.length} 个会话等待处理
            <Icon name="chevron" size={13} />
          </button>
        )}
      </div>
      <section className="servers-panel">
        <div className="panel-heading">
          <h2>我的服务器</h2>
          <span>{hosts.length} 台</span>
        </div>
        {hosts.length ? (
          <table className="server-table">
            <thead>
              <tr>
                <th>服务器</th>
                <th>登录用户</th>
                <th>Agent 访问</th>
                <th>
                  <span className="sr-only">操作</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {hosts.map((h) => {
                const busy = pending.includes(`connect:${h.id}`);
                return (
                  <tr key={h.id}>
                    <td>
                      <div className="server-name">
                        <span className="server-tile">
                          <Icon name="server" size={21} />
                        </span>
                        <div>
                          <strong title={h.name}>{h.name}</strong>
                          <code>
                            <span className="endpoint-user">{h.username}@</span>
                            {h.host}
                            <span>:{h.port}</span>
                          </code>
                        </div>
                      </div>
                    </td>
                    <td>
                      <span className="user-name">{h.username}</span>
                    </td>
                    <td>
                      <span
                        className={`badge ${h.agent_access ? "agent" : "neutral"}`}
                      >
                        <Icon
                          name={h.agent_access ? "link" : "shield"}
                          size={13}
                        />
                        {h.agent_access ? "已允许" : "仅自己使用"}
                      </span>
                    </td>
                    <td className="row-actions">
                      <button onClick={() => void onConnect(h)} disabled={busy}>
                        <Icon name="terminal" size={15} />
                        {busy ? "连接中…" : "连接"}
                      </button>
                      <button
                        className="icon-button"
                        aria-label={`编辑 ${h.name}`}
                        onClick={() => onEdit(h)}
                      >
                        <Icon name="settings" size={17} />
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        ) : (
          <div className="empty-block">
            <Icon name={query ? "search" : "server"} size={32} />
            <h3>{query ? "没有找到匹配的服务器" : "从你的第一台服务器开始"}</h3>
            <p>
              {query
                ? "试试服务器名称、地址或用户名。"
                : "添加连接资料并核对指纹，即可打开终端。"}
            </p>
            {!query && (
              <button onClick={() => onEdit("new")}>
                <Icon name="plus" size={16} />
                添加服务器
              </button>
            )}
          </div>
        )}
      </section>
      <section className="sessions-panel">
        <div className="panel-heading">
          <h2>当前会话</h2>
          <span>点击继续操作</span>
        </div>
        {data.spaces.length ? (
          <div className="session-list">
            {[...data.spaces].reverse().map((s) => {
              const last = s.jobs.at(-1);
              return (
                <button
                  key={s.space}
                  className="session-item"
                  onClick={() => onSelect(s.space)}
                >
                  <span className="session-icon" data-kind={s.origin}>
                    <Icon
                      name={s.origin === "agent" ? "link" : "terminal"}
                      size={18}
                    />
                  </span>
                  <div>
                    <strong>
                      {s.origin === "agent" ? s.name : `${s.host_name} · 终端`}
                    </strong>
                    <small>
                      {s.control.note ??
                        (s.origin === "agent"
                          ? `${s.actor} · ${s.host_name}`
                          : "自己的终端，仅你可操作")}
                    </small>
                  </div>
                  <span className="session-status">
                    <span
                      className="dot"
                      data-status={
                        !s.connected ? "unknown" : (last?.status ?? "succeeded")
                      }
                    />
                    {!s.connected
                      ? "连接已断开"
                      : s.control.note
                        ? "等待处理"
                        : last
                          ? STATUS[last.status]
                          : "已连接"}
                    {failedJobs(s) > 0 && (
                      <span className="failure-note">
                        {" "}
                        · {failedJobs(s)} 项失败
                      </span>
                    )}
                  </span>
                  <Icon name="chevron" size={15} />
                </button>
              );
            })}
          </div>
        ) : (
          <p className="empty-sessions">
            还没有会话。点击上方的“连接”打开终端。
          </p>
        )}
      </section>
      <p className="workspace-footnote">
        <Icon name="shield" size={13} />
        终端、文件与服务器状态，在同一个工作空间。
      </p>
    </div>
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
  const [clear, setClear] = useState(0);
  const held = space.control.owner === "human";
  const byAgent = space.origin === "agent";
  const id = space.space;
  const busy = pending.includes(`space:${id}`);
  const writable = held && space.connected && job?.status === "running";
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
          {byAgent && space.connected && (
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
      {space.control.note && (
        <div className="handoff" role="status">
          <Icon name="alert" size={18} />
          <div>
            <strong>需要你处理</strong>
            <p>{space.control.note}</p>
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
      <nav className="session-pages" aria-label="空间页面">
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
              <div className="terminal-tools">
                <span>
                  <Icon name="terminal" size={14} />
                  {job.pty ? "交互终端" : "命令输出"}
                </span>
                <div>
                  <button
                    aria-label="缩小终端字号"
                    disabled={fontSize <= 11}
                    onClick={() => setFontSize(fontSize - 1)}
                  >
                    A−
                  </button>
                  <span>{fontSize}px</span>
                  <button
                    aria-label="放大终端字号"
                    disabled={fontSize >= 20}
                    onClick={() => setFontSize(fontSize + 1)}
                  >
                    A+
                  </button>
                  <i />
                  <button onClick={() => setClear((n) => n + 1)}>
                    清空显示
                  </button>
                  {job.command && (
                    <button
                      title="复制完整命令"
                      onClick={() => void copy(job.command!)}
                    >
                      <Icon name="copy" size={14} />
                      复制命令
                    </button>
                  )}
                </div>
              </div>
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
                    clear={clear}
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
