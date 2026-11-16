import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useState } from "react";
import { call, explain, type Host, type Job, type Snapshot, type Space } from "./api";
import { HostForm } from "./HostForm";
import { Terminal } from "./Terminal";

const STATUS: Record<Job["status"], string> = {
  running: "运行中",
  succeeded: "成功",
  failed: "失败",
  unknown: "结果未知",
};

export function App() {
  const [data, setData] = useState<Snapshot>();
  const [selected, setSelected] = useState<string>();
  const [picked, setPicked] = useState<Record<string, string>>({});
  const [editing, setEditing] = useState<Host | "new">();
  const [notice, setNotice] = useState<string>();
  const [quitting, setQuitting] = useState(false);

  const refresh = useCallback(() => {
    call<Snapshot>("snapshot").then(setData, (error) => setNotice(explain(error)));
  }, []);

  useEffect(() => {
    refresh();
    const stop = listen("changed", refresh);
    return () => void stop.then((off) => off());
  }, [refresh]);

  const running = data?.spaces.flatMap((s) => s.jobs).filter((j) => j.status === "running").length ?? 0;
  useEffect(() => {
    // Closing the window ends every connection, so ask first while something is running.
    const stop = getCurrentWindow().onCloseRequested((event) => {
      if (running > 0) {
        event.preventDefault();
        setQuitting(true);
      }
    });
    return () => void stop.then((off) => off());
  }, [running]);

  const act = (work: Promise<unknown>) => work.catch((error) => setNotice(explain(error)));

  if (!data) return <main className="loading">正在启动…</main>;

  const space = data.spaces.find((s) => s.space === selected) ?? data.spaces.at(-1);
  const agents = data.spaces.filter((s) => s.origin === "agent");
  const mine = data.spaces.filter((s) => s.origin === "human");
  const openTerminal = (host: Host) =>
    act(call<{ space: string }>("open_terminal", { host: host.id }).then((opened) => setSelected(opened.space)));

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="titlebar" data-tauri-drag-region />
        <nav>
          <Section title="服务器">
            {data.hosts.map((host) => (
              <div className="row" key={host.id}>
                <button className="row-main" onClick={() => openTerminal(host)} title="打开终端">
                  <span className="row-name">{host.name}</span>
                  <span className="row-meta">
                    {host.username}@{host.host}
                  </span>
                </button>
                {host.agent_access && <span className="tag agent">Agent 可用</span>}
                <button className="quiet" onClick={() => setEditing(host)}>
                  设置
                </button>
              </div>
            ))}
            <button className="add" onClick={() => setEditing("new")}>
              添加服务器
            </button>
          </Section>
          {agents.length > 0 && (
            <Section title="Agent 的任务">
              {agents.map((s) => (
                <SpaceRow key={s.space} space={s} active={s.space === space?.space} onSelect={setSelected} />
              ))}
            </Section>
          )}
          {mine.length > 0 && (
            <Section title="我的终端">
              {mine.map((s) => (
                <SpaceRow key={s.space} space={s} active={s.space === space?.space} onSelect={setSelected} />
              ))}
            </Section>
          )}
        </nav>
        <footer>
          {data.skill.skill && data.skill.cli ? (
            <p className="hint">Agent 已可以使用 vibe-ssh。</p>
          ) : (
            <button
              className="primary"
              onClick={() =>
                act(
                  call<{ skipped: { path: string }[] }>("install_skill").then((report) => {
                    refresh();
                    if (report.skipped.length > 0) {
                      setNotice(`这些位置已有其他同名内容，没有覆盖：${report.skipped.map((s) => s.path).join("、")}`);
                    }
                  }),
                )
              }
            >
              让 Agent 可以使用
            </button>
          )}
        </footer>
      </aside>

      <main className="main">
        {space ? (
          <SpaceView
            key={space.space}
            space={space}
            job={space.jobs.find((j) => j.job === picked[space.space]) ?? space.jobs.at(-1)}
            onPick={(job) => setPicked({ ...picked, [space.space]: job })}
            act={act}
          />
        ) : (
          <Welcome data={data} />
        )}
      </main>

      {editing && (
        <HostForm
          initial={editing === "new" ? undefined : editing}
          onClose={() => {
            setEditing(undefined);
            refresh();
          }}
        />
      )}
      {quitting && (
        <div className="scrim">
          <div className="dialog" role="alertdialog" aria-labelledby="quit-title">
            <h2 id="quit-title">还有 {running} 个任务在运行</h2>
            <p>退出后所有连接会断开，Agent 也无法继续。服务器上已经启动的程序可能还会继续运行。</p>
            <div className="actions">
              <button onClick={() => setQuitting(false)}>继续运行</button>
              <button className="danger" onClick={() => getCurrentWindow().destroy()}>
                退出
              </button>
            </div>
          </div>
        </div>
      )}
      {notice && (
        <div className="notice" role="alert">
          <span>{notice}</span>
          <button className="quiet" onClick={() => setNotice(undefined)}>
            知道了
          </button>
        </div>
      )}
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section>
      <h2>{title}</h2>
      {children}
    </section>
  );
}

function SpaceRow({ space, active, onSelect }: { space: Space; active: boolean; onSelect: (id: string) => void }) {
  const busy = space.jobs.some((j) => j.status === "running");
  return (
    <button className="row space" data-active={active} onClick={() => onSelect(space.space)}>
      <span className="pen" data-owner={space.control.owner} data-busy={busy} />
      <span className="row-main">
        <span className="row-name">{space.origin === "agent" ? space.name : space.host_name}</span>
        <span className="row-meta">
          {space.origin === "agent" ? `${space.actor}，${space.host_name}` : space.connected ? "已连接" : "已断开"}
        </span>
      </span>
      {space.control.note && <span className="tag human">等你处理</span>}
    </button>
  );
}

type ViewProps = {
  space: Space;
  job: Job | undefined;
  onPick: (job: string) => void;
  act: (work: Promise<unknown>) => Promise<unknown>;
};

function SpaceView({ space, job, onPick, act }: ViewProps) {
  const [closing, setClosing] = useState(false);
  const id = space.space;
  const byAgent = space.origin === "agent";
  const held = space.control.owner === "human";

  return (
    <>
      <header className="control" data-owner={space.control.owner} data-tauri-drag-region>
        <div className="who" data-tauri-drag-region>
          <h1>{byAgent ? space.name : space.host_name}</h1>
          <p>
            {!byAgent
              ? "你的终端，Agent 看不到这里。"
              : held
                ? `你正在操作，${space.actor} 已暂停。`
                : `${space.actor} 正在操作 ${space.host_name}，你可以随时接管。`}
            {!space.connected && " 连接已断开。"}
          </p>
        </div>
        <div className="actions">
          {byAgent && !held && (
            <button className="take" onClick={() => act(call("takeover", { space: id }))}>
              接管
            </button>
          )}
          {held && (
            <button onClick={() => act(call<string>("shell", { space: id }).then(onPick))}>新开终端</button>
          )}
          {byAgent && held && (
            <button className="give" onClick={() => act(call("release", { space: id }))}>
              交还给 {space.actor}
            </button>
          )}
          {closing ? (
            <button className="danger" onClick={() => act(call("close_space", { space: id }))} onBlur={() => setClosing(false)}>
              确认关闭
            </button>
          ) : (
            <button className="quiet" onClick={() => setClosing(true)}>
              关闭
            </button>
          )}
        </div>
      </header>

      {space.control.note && (
        <p className="handoff">
          {space.actor} 请你处理：{space.control.note}
        </p>
      )}

      {space.jobs.length > 1 && (
        <div className="tabs" role="tablist">
          {space.jobs.map((j) => (
            <button key={j.job} role="tab" aria-selected={j.job === job?.job} onClick={() => onPick(j.job)}>
              <span className="state" data-status={j.status} title={STATUS[j.status]} />
              {j.command ? j.command.split("\n")[0].slice(0, 28) : "终端"}
            </button>
          ))}
        </div>
      )}

      {job ? (
        <>
          <Terminal key={job.job} space={id} job={job.job} pty={job.pty} writable={held && job.status === "running"} />
          <div className="statusbar">
            <span>
              {STATUS[job.status]}
              {job.exit_code !== null && `，退出码 ${job.exit_code}`}
            </span>
            {held && job.status === "running" && !job.pty && (
              <button className="quiet" onClick={() => act(call("job_interrupt", { space: id, job: job.job }))}>
                中断这个任务
              </button>
            )}
          </div>
        </>
      ) : (
        <p className="empty">{byAgent ? `${space.actor} 还没有在这里运行任何命令。` : "这个空间里没有终端。"}</p>
      )}
    </>
  );
}

function Welcome({ data }: { data: Snapshot }) {
  const steps: [boolean, string, string][] = [
    [data.hosts.length > 0, "添加一台服务器", "填写地址和登录方式，并核对服务器指纹。点左侧的服务器名可以直接打开终端。"],
    [data.hosts.some((h) => h.agent_access), "允许 Agent 使用它", "在服务器设置里打开。没有打开的服务器，Agent 看不到。"],
    [data.skill.skill && data.skill.cli, "让 Agent 可以使用", "点左下角的按钮，为 Codex、Claude Code 等安装 Skill 和 vibe-ssh 命令。"],
    [false, "把任务交给你的 Agent", "例如对它说：用 vibe-ssh 把这个项目部署到我的服务器。它的操作会出现在左侧，你可以随时接管。"],
  ];
  return (
    <div className="welcome" data-tauri-drag-region>
      <h1>你和 Agent 共用的 SSH</h1>
      <ol>
        {steps.map(([done, title, detail]) => (
          <li key={title} data-done={done}>
            <strong>{title}</strong>
            <span>{detail}</span>
          </li>
        ))}
      </ol>
    </div>
  );
}
