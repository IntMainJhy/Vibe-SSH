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

const Terminal = lazy(() =>
  import("./Terminal").then((module) => ({ default: module.Terminal })),
);
const STATUS: Record<Job["status"], string> = {
  running: "运行中",
  succeeded: "已完成",
  failed: "失败",
  unknown: "结果未知",
};
type Notice = { text: string; kind: "error" | "success" };
type Act = (key: string, work: () => Promise<unknown>) => Promise<void>;

export function App() {
  const { data, error, refreshing, refresh } = useWorkspace();
  const preferences = usePreferences();
  const [selected, select] = useState<string | null>(null);
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
  runningRef.current = running;
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
  const install = () =>
    act("install", async () => {
      const report = await call<{ skipped: { path: string }[] }>(
        "install_skill",
      );
      if (report.skipped.length)
        notify(
          `以下位置已有同名内容，已保留：${report.skipped.map((s) => s.path).join("、")}`,
        );
      else setNotice({ text: "Skill 和 vibe-ssh 命令已安装", kind: "success" });
    });

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="traffic-space" data-tauri-drag-region />
        <div className="brand">
          <span className="brand-mark">
            <Icon name="terminal" size={22} />
          </span>
          <strong>
            Vibe<span>SSH</span>
          </strong>
          <span className="version">{data?.status.version ?? "0.1.0"}</span>
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
                          ? "Agent 操作"
                          : "我的终端"}
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
          <div className="integration">
            <span
              className={`integration-icon ${data?.skill.skill && data.skill.cli ? "ready" : ""}`}
            >
              <Icon name="link" size={18} />
            </span>
            <div>
              <strong>Agent 接入</strong>
              <small>
                {data?.skill.skill && data.skill.cli
                  ? "Skill 与命令已就绪"
                  : "通过 Skill 使用 SSH"}
              </small>
            </div>
            <button
              className="text-button"
              disabled={pending.includes("install")}
              onClick={() => void install()}
            >
              {pending.includes("install")
                ? "安装中"
                : data?.skill.skill && data.skill.cli
                  ? "更新"
                  : "启用"}
            </button>
          </div>
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
              {error ? "服务异常" : data ? "本地运行" : "连接中"}
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
            本机 SSH 客户端
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
            <p>{error ? "无法加载工作台" : "正在连接本地服务…"}</p>
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
            onInstall={install}
          />
        )}
      </main>
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
            还有 {running} 个任务在运行。退出会断开所有连接，Agent
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
  onInstall,
}: {
  data: Snapshot;
  hosts: Host[];
  query: string;
  pending: string[];
  onConnect: (h: Host) => Promise<void>;
  onEdit: (h: Host | "new") => void;
  onSelect: (s: string) => void;
  onInstall: () => Promise<void>;
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
          <p>连接服务器，继续手头的工作。</p>
        </div>
        <button className="primary" onClick={() => onEdit("new")}>
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
                          <strong>{h.name}</strong>
                          <code>
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
      <aside className="agent-help">
        <span className="help-icon">
          <Icon name="link" size={21} />
        </span>
        <div>
          <h3>也可以交给你的 Agent</h3>
          <p>
            启用 Skill 后，让 Agent 通过 Vibe-SSH
            部署或排障。执行过程会出现在会话里，你随时可以接管。
          </p>
        </div>
        {data.skill.cli && data.skill.skill ? (
          <code>vibe-ssh skill</code>
        ) : (
          <button
            disabled={pending.includes("install")}
            onClick={() => void onInstall()}
          >
            {pending.includes("install") ? "正在启用…" : "启用 Agent 接入"}
            <Icon name="arrow" size={15} />
          </button>
        )}
      </aside>
      <p className="workspace-footnote">
        <Icon name="shield" size={13} />
        连接由本机管理，凭据保存在这台 Mac。
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
                void act(`space:${id}`, async () =>
                  onPick(await call<string>("shell", { space: id })),
                )
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
      <div className="terminal-shell">
        <div className="terminal-tabs" role="tablist" aria-label="会话中的任务">
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
                if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key))
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
                {j.command ? j.command.split("\n")[0].slice(0, 42) : "交互终端"}
              </span>
              {!j.pty && <span className="tab-kind">任务</span>}
            </button>
          ))}
          <span className="tab-spacer" />
          <span className="terminal-mode">{writable ? "可输入" : "只读"}</span>
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
                <button onClick={() => setClear((n) => n + 1)}>清空显示</button>
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
                  : !held
                    ? "只读观察 · 接管后可输入"
                    : job.status !== "running"
                      ? "任务输出已保留"
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
