import type { Host, Job, Snapshot, Space } from "./api";
import { Icon, type IconName } from "./Icon";

export type HomeView = "overview" | "servers" | "sessions";
export const STATUS: Record<Job["status"], string> = {
  running: "运行中",
  succeeded: "已完成",
  failed: "失败",
  unknown: "结果未知",
};
export const failedJobs = (space: Space) =>
  space.jobs.filter((job) => job.status === "failed").length;
type Open = (
  space: string,
  page?: "terminal" | "files" | "resources",
  job?: string,
) => void;
const spaceName = (s: Space) => (s.origin === "human" ? s.host_name : s.name);
const connectionState = (s: Space) =>
  !s.connected
    ? "连接已断开"
    : s.control.note
      ? "等待你处理"
      : s.control.owner === "human"
        ? s.origin === "human"
          ? "我的终端"
          : "你已接管"
        : "Agent 操作中";

export function Workbench({
  data,
  hosts,
  sessions,
  query,
  view,
  pending,
  onConnect,
  onEdit,
  onSelect,
  onView,
}: {
  data: Snapshot;
  hosts: Host[];
  sessions: Space[];
  query: string;
  view: HomeView;
  pending: string[];
  onConnect: (host: Host) => Promise<void>;
  onEdit: (host: Host | "new") => void;
  onSelect: Open;
  onView: (view: HomeView) => void;
}) {
  const connected = data.spaces.filter((s) => s.connected);
  const running = data.spaces
    .flatMap((s) => s.jobs)
    .filter((j) => j.status === "running").length;
  const featured =
    [...connected].reverse().find((s) => s.origin === "human") ??
    connected.at(-1);
  const searching = !!query.trim();
  const overview = !searching && view === "overview";
  const attention = data.spaces.flatMap<{
    space: Space;
    title: string;
    detail: string;
    job: string | undefined;
  }>((s) => {
    const failed = [...s.jobs].reverse().find((j) => j.status === "failed");
    if (s.control.note)
      return [
        {
          space: s,
          title: "需要你处理",
          detail: s.control.note,
          job: undefined,
        },
      ];
    if (!s.connected)
      return [
        {
          space: s,
          title: "连接已断开",
          detail: "请核实远端结果，再手动连接",
          job: undefined,
        },
      ];
    if (failed)
      return [
        {
          space: s,
          title: "任务执行失败",
          detail: failed.command ?? "交互终端",
          job: failed.job,
        },
      ];
    return [];
  });
  return (
    <div className="cloud-workbench" data-overview={overview}>
      {overview && (
        <>
          <section className="overview-metrics" aria-label="工作空间概览">
            <OverviewMetric
              icon="server"
              label="已配置服务器"
              value={data.hosts.length}
              detail="连接资料由你管理"
            />
            <OverviewMetric
              icon="terminal"
              label="已连接会话"
              value={connected.length}
              detail={`${data.spaces.length} 个会话空间`}
            />
            <OverviewMetric
              icon="activity"
              label="运行中任务"
              value={running}
              detail="含交互终端"
            />
            <OverviewMetric
              icon="link"
              label="允许 Agent 接入"
              value={data.hosts.filter((h) => h.agent_access).length}
              detail="按服务器单独授权"
            />
          </section>
          <div className="overview-middle">
            <section
              className="focus-space"
              aria-labelledby="focus-title"
              data-empty={!featured}
            >
              <div className="focus-top">
                <span>
                  <span className="focus-led" />
                  {featured ? "继续工作" : "SSH 工作空间"}
                </span>
                <Icon name="terminal" size={20} />
              </div>
              <div className="focus-body">
                <span className="focus-eyebrow">
                  {featured ? connectionState(featured) : "连接你的服务器"}
                </span>
                <h2
                  id="focus-title"
                  title={featured ? spaceName(featured) : undefined}
                >
                  {featured ? spaceName(featured) : "从一台服务器开始"}
                </h2>
                <p>
                  {featured
                    ? `${featured.host_name} · ${featured.origin === "human" ? "个人空间" : featured.actor}`
                    : "终端、文件和资源状态，在同一个工作空间。"}
                </p>
              </div>
              {featured ? (
                <>
                  <div className="focus-stats">
                    <div>
                      <span>当前任务</span>
                      <strong>
                        {featured.jobs.length}
                        <small>项</small>
                      </strong>
                    </div>
                    <div>
                      <span>运行中</span>
                      <strong>
                        {
                          featured.jobs.filter((j) => j.status === "running")
                            .length
                        }
                        <small>项</small>
                      </strong>
                    </div>
                    <div>
                      <span>控制权</span>
                      <strong className="focus-owner">
                        {featured.control.owner === "human"
                          ? "由你操作"
                          : "Agent"}
                      </strong>
                    </div>
                  </div>
                  <button
                    className="focus-action"
                    onClick={() => onSelect(featured.space)}
                  >
                    打开会话
                    <Icon name="arrow" size={16} />
                  </button>
                </>
              ) : (
                <button
                  className="focus-action"
                  onClick={() =>
                    data.hosts.length ? onView("servers") : onEdit("new")
                  }
                >
                  {data.hosts.length ? "选择服务器" : "添加服务器"}
                  <Icon name="arrow" size={16} />
                </button>
              )}
            </section>
            <section
              className="cloud-panel live-sessions"
              aria-label="当前会话"
            >
              <PanelHeading
                title="当前会话"
                detail={`${data.spaces.length} 个空间`}
                action={() => onView("sessions")}
              />
              <SessionList
                sessions={[...data.spaces].reverse().slice(0, 3)}
                onSelect={onSelect}
                compact
              />
            </section>
            <section
              className="cloud-panel quick-actions"
              aria-labelledby="quick-title"
            >
              <h2 id="quick-title">快捷操作</h2>
              <QuickAction
                icon="plus"
                title="添加服务器"
                detail="配置连接与验证指纹"
                onClick={() => onEdit("new")}
              />
              {featured ? (
                <>
                  <QuickAction
                    icon="folder"
                    title="管理文件"
                    detail="浏览、传输与编辑"
                    onClick={() => onSelect(featured.space, "files")}
                  />
                  <QuickAction
                    icon="activity"
                    title="资源监控"
                    detail="CPU、内存与存储"
                    onClick={() => onSelect(featured.space, "resources")}
                  />
                </>
              ) : (
                <QuickAction
                  icon="server"
                  title="服务器列表"
                  detail="选择服务器打开终端"
                  onClick={() => onView("servers")}
                />
              )}
            </section>
          </div>
        </>
      )}
      {searching && (
        <p className="search-results-heading">
          “{query}” · {hosts.length} 台服务器，{sessions.length} 个会话
        </p>
      )}
      <div className="overview-bottom" data-single={!overview}>
        {(overview || searching || view === "servers") && (
          <section
            className="cloud-panel host-directory"
            aria-label="服务器列表"
          >
            <PanelHeading
              title={searching ? "匹配的服务器" : "服务器"}
              detail={`${hosts.length} 台`}
            />
            {hosts.length ? (
              <div className="server-table-scroll">
                <table className="server-table cloud-server-table">
                  <thead>
                    <tr>
                      <th>服务器 / 连接地址</th>
                      <th>登录用户</th>
                      <th>Agent 接入</th>
                      <th>
                        <span className="sr-only">操作</span>
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {hosts.map((h) => (
                      <tr key={h.id}>
                        <td>
                          <div className="server-name">
                            <span className="server-tile">
                              <Icon name="server" size={19} />
                            </span>
                            <div>
                              <strong title={h.name}>{h.name}</strong>
                              <code>
                                <span className="endpoint-user">
                                  {h.username}@
                                </span>
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
                          <button
                            aria-label={`连接 ${h.name}`}
                            disabled={pending.includes(`connect:${h.id}`)}
                            onClick={() => void onConnect(h)}
                          >
                            <Icon name="terminal" size={14} />
                            {pending.includes(`connect:${h.id}`)
                              ? "连接中…"
                              : "连接"}
                          </button>
                          <button
                            className="icon-button"
                            aria-label={`编辑 ${h.name}`}
                            onClick={() => onEdit(h)}
                          >
                            <Icon name="settings" size={16} />
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <div className="cloud-empty">
                <Icon name={searching ? "search" : "server"} size={28} />
                <h3>
                  {searching ? "没有匹配的服务器" : "添加你的第一台服务器"}
                </h3>
                <p>
                  {searching
                    ? "试试名称、地址或用户名。"
                    : "填写连接资料并核对主机指纹，即可使用。"}
                </p>
                {!searching && (
                  <button className="primary" onClick={() => onEdit("new")}>
                    <Icon name="plus" size={15} />
                    添加服务器
                  </button>
                )}
              </div>
            )}
          </section>
        )}
        {overview && (
          <section
            className="cloud-panel attention-panel"
            aria-label="需要关注的会话"
          >
            <PanelHeading
              title="需要关注"
              detail={`${attention.length} 个会话`}
            />
            {attention.length ? (
              <div className="attention-list">
                {attention.slice(0, 4).map((a) => (
                  <button
                    key={a.space.space}
                    onClick={() => onSelect(a.space.space, "terminal", a.job)}
                  >
                    <span className="attention-symbol">
                      <Icon name="alert" size={15} />
                    </span>
                    <span>
                      <strong>{a.title}</strong>
                      <small title={a.detail}>
                        {spaceName(a.space)} · {a.detail}
                      </small>
                    </span>
                    <Icon name="chevron" size={14} />
                  </button>
                ))}
              </div>
            ) : (
              <div className="attention-clear">
                <Icon name="check" size={22} />
                <strong>暂无需要处理的会话</strong>
                <p>连接和任务状态会在这里更新。</p>
              </div>
            )}
          </section>
        )}
        {(searching || view === "sessions") && (
          <section
            className="cloud-panel session-directory"
            aria-label="会话列表"
          >
            <PanelHeading
              title={searching ? "匹配的会话" : "当前会话"}
              detail={`${sessions.length} 个空间`}
            />
            <SessionList
              sessions={[...sessions].reverse()}
              onSelect={onSelect}
            />
          </section>
        )}
      </div>
      <footer className="cloud-footer">
        <span>
          <Icon name="shield" size={13} />
          SSH 工作空间
        </span>
        <span>Vibe-SSH {data.status.version}</span>
      </footer>
    </div>
  );
}
function OverviewMetric({
  icon,
  label,
  value,
  detail,
}: {
  icon: IconName;
  label: string;
  value: number;
  detail: string;
}) {
  return (
    <article className="overview-metric">
      <span className="metric-icon">
        <Icon name={icon} size={18} />
      </span>
      <div>
        <h2>{label}</h2>
        <strong>{value.toString().padStart(2, "0")}</strong>
      </div>
      <p>{detail}</p>
    </article>
  );
}
function PanelHeading({
  title,
  detail,
  action,
}: {
  title: string;
  detail: string;
  action?: () => void;
}) {
  return (
    <div className="cloud-panel-heading">
      <h2>{title}</h2>
      {action ? (
        <button className="panel-link" onClick={action}>
          {detail}
          <Icon name="chevron" size={12} />
        </button>
      ) : (
        <span>{detail}</span>
      )}
    </div>
  );
}
function QuickAction({
  icon,
  title,
  detail,
  onClick,
}: {
  icon: IconName;
  title: string;
  detail: string;
  onClick: () => void;
}) {
  return (
    <button className="quick-action" onClick={onClick}>
      <span className="quick-icon">
        <Icon name={icon} size={16} />
      </span>
      <span>
        <strong>{title}</strong>
        <small>{detail}</small>
      </span>
      <Icon name="chevron" size={13} />
    </button>
  );
}
function SessionList({
  sessions,
  onSelect,
  compact = false,
}: {
  sessions: Space[];
  onSelect: Open;
  compact?: boolean;
}) {
  if (!sessions.length)
    return (
      <div className="cloud-empty small-empty">
        <Icon name="terminal" size={25} />
        <h3>还没有会话</h3>
        <p>连接服务器后，从这里继续工作。</p>
      </div>
    );
  return (
    <div className="cloud-session-list" data-compact={compact}>
      {sessions.map((s) => {
        const last = s.jobs.at(-1);
        return (
          <button
            key={s.space}
            className="cloud-session"
            onClick={() => onSelect(s.space)}
          >
            <span className="session-icon">
              <Icon
                name={s.origin === "human" ? "terminal" : "link"}
                size={17}
              />
            </span>
            <span className="cloud-session-text">
              <strong>{spaceName(s)}</strong>
              <small>
                {s.origin === "agent"
                  ? `${s.host_name} · ${connectionState(s)}`
                  : connectionState(s)}
              </small>
            </span>
            <span className="cloud-session-status">
              <span
                className="dot"
                data-status={
                  !s.connected ? "unknown" : (last?.status ?? "succeeded")
                }
              />
              {!s.connected
                ? "已断开"
                : s.control.note
                  ? "待处理"
                  : last
                    ? STATUS[last.status]
                    : "已连接"}
              {failedJobs(s) > 0 && <small>{failedJobs(s)} 项失败</small>}
            </span>
            <Icon name="chevron" size={14} />
          </button>
        );
      })}
    </div>
  );
}
