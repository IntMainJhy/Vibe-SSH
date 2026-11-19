import { useEffect, useRef, useState } from "react";
import { call, explain, type JobStatus } from "./api";
import { STATUS } from "./Workbench";
import { Icon } from "./Icon";
import { bytes } from "./Transfers";

const transferLabels = {
  upload: "上传文件",
  download: "下载文件",
  upload_folder: "上传文件夹",
  download_folder: "下载文件夹",
};
const stepLabels = {
  pending: "待提交", running: "运行中", succeeded: "已完成",
  failed: "失败", unknown: "结果未知", skipped: "未提交",
};
type DeploymentStep = {
  name: string; kind: "run" | "put"; status: keyof typeof stepLabels;
  job_record: string | null; exit_code: number | null; reason: string | null;
};

type TaskRecord = {
  record: string;
  host_name: string;
  space_name: string;
  actor: string;
  origin: "agent" | "human";
  command: string | null;
  cwd?: string | null;
  command_truncated: boolean;
  pty: boolean;
  started_unix_ms: number;
  elapsed_ms: number;
  status: JobStatus;
  reason: string | null;
  exit_code: number | null;
  transfer?: { kind: keyof typeof transferLabels; remote: string; size: number | null };
  deployment?: DeploymentStep[];
  archive?: { stdout_bytes: number; stderr_bytes: number; issue: string | null };
};
const title = (record: TaskRecord) => {
  if (record.deployment?.length) return `Agent 部署 · ${record.space_name}`;
  if (!record.transfer) return record.command ?? "交互终端";
  const { kind, remote } = record.transfer;
  const name = remote.replace(/\/$/, "").split("/").at(-1) || "/";
  return `${transferLabels[kind]} · ${name}`;
};
type Stream = {
  text: string;
  total_bytes: number;
  truncated: boolean;
  invalid_utf8: boolean;
};
type Detail = TaskRecord & { stdout: Stream; stderr: Stream };
type Page = { records: TaskRecord[]; more: boolean };
type LogPage = {
  text: string;
  offset: number;
  next_offset: number;
  archived_bytes: number;
  observed_bytes: number;
  complete: boolean;
  invalid_utf8: boolean;
  issue: string | null;
};

function ArchivedLog({ record, stream }: { record: string; stream: "stdout" | "stderr" }) {
  const [offsets, setOffsets] = useState([0]);
  const offset = offsets.at(-1)!;
  const [page, setPage] = useState<LogPage>();
  const [reload, setReload] = useState(0);
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    let active = true;
    setBusy(true);
    setError("");
    void call<LogPage>("history_log", { record, stream, offset }).then(
      (next) => { if (active) setPage(next); },
      (reason) => { if (active) setError(explain(reason)); },
    ).finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [record, stream, offset, reload]);
  const save = async () => {
    setSaving(true);
    setError("");
    setSaved(false);
    try {
      setSaved(await call<boolean>("export_history_log", { record, stream }));
    } catch (reason) {
      setError(explain(reason));
    } finally {
      setSaving(false);
    }
  };
  return (
    <section aria-label="归档日志" aria-busy={busy}>
      <div className="history-log-actions">
        <button disabled={busy || offsets.length < 2} onClick={() => setOffsets((old) => old.slice(0, -1))}>上一段</button>
        <button disabled={busy || !page || page.next_offset >= page.archived_bytes}
          onClick={() => page && setOffsets((old) => [...old, page.next_offset])}>下一段</button>
        <button disabled={busy} onClick={() => setReload((n) => n + 1)}>刷新日志</button>
        <button disabled={!page || saving} onClick={() => void save()}>保存原始日志</button>
      </div>
      {page && <p className="history-note">已归档 {bytes(page.archived_bytes)} / 已收到 {bytes(page.observed_bytes)} · {page.complete ? "完整" : "尚未确认完整"}</p>}
      {page?.issue && <p className="history-note">{page.issue}</p>}
      {page?.invalid_utf8 && <p className="history-note">包含非 UTF-8 字节。保存原始日志可保留原始内容。</p>}
      {error && <p className="form-error" role="alert">{error}</p>}
      {saved && <p role="status">日志已保存。</p>}
      <pre id="history-output" className="history-output" role="tabpanel" aria-labelledby={`history-${stream}-tab`} tabIndex={0}>
        {page?.text || (busy ? "正在读取日志…" : "此路暂时没有归档内容。")}
      </pre>
    </section>
  );
}

export function History({ revision }: { revision: number }) {
  const [page, setPage] = useState<Page>();
  const [offset, setOffset] = useState(0);
  const [reload, setReload] = useState(0);
  const [selected, setSelected] = useState<string>();
  const [detail, setDetail] = useState<Detail>();
  const [listError, setListError] = useState("");
  const [detailError, setDetailError] = useState("");
  const [listBusy, setListBusy] = useState(false);
  const [detailBusy, setDetailBusy] = useState(false);
  const [showDetail, setShowDetail] = useState(false);
  const [stream, setStream] = useState<"stdout" | "stderr">("stdout");
  const [archiveRecord, setArchiveRecord] = useState<string>();
  const detailFor = useRef<string | undefined>(undefined);
  useEffect(() => {
    let active = true;
    setListBusy(true);
    setListError("");
    void call<Page>("history_list", { offset, limit: 50 })
      .then((next) => {
        if (!active) return;
        setPage(next);
        setSelected((old) =>
          next.records.some((r) => r.record === old)
            ? old
            : next.records[0]?.record,
        );
      })
      .catch((e) => {
        if (active) setListError(explain(e));
      })
      .finally(() => {
        if (active) setListBusy(false);
      });
    return () => {
      active = false;
    };
  }, [offset, reload, revision]);
  useEffect(() => {
    if (!selected) {
      setDetail(undefined);
      return;
    }
    let active = true;
    setDetail((old) => (old?.record === selected ? old : undefined));
    setDetailError("");
    setDetailBusy(true);
    void call<Detail>("history_get", { record: selected, tail: 65_536 })
      .then((next) => {
        if (active) {
          setDetail(next);
          if (detailFor.current !== next.record)
            setStream(next.stdout.text ? "stdout" : "stderr");
          detailFor.current = next.record;
        }
      })
      .catch((e) => {
        if (active) setDetailError(explain(e));
      })
      .finally(() => {
        if (active) setDetailBusy(false);
      });
    return () => {
      active = false;
    };
  }, [selected, reload, revision]);
  const metadata = detail ?? page?.records.find((r) => r.record === selected);
  const output = detail?.[stream];
  return (
    <section className="history-page" data-detail={showDetail}>
      <header className="history-intro">
        <div>
          <p>
            保留最近 256 条已结束的部署、命令和文件传输记录。
          </p>
        </div>
        <button
          disabled={listBusy || detailBusy}
          onClick={() => setReload((n) => n + 1)}
        >
          <Icon name="refresh" size={15} />
          重新加载记录
        </button>
      </header>
      {listError && (
        <p className="form-error" role="alert">
          {listError}
        </p>
      )}
      <div className="history-layout">
        <div className="history-list-panel" aria-busy={listBusy}>
          <div className="history-list" aria-label="历史任务">
            {page?.records.map((record) => (
              <button
                key={record.record}
                className="history-record"
                aria-pressed={record.record === selected}
                onClick={() => {
                  setSelected(record.record);
                  setShowDetail(true);
                }}
              >
                <span>
                  <strong title={record.host_name}>{record.host_name}</strong>
                  <small>{STATUS[record.status]}</small>
                </span>
                <code title={title(record)}>
                  {title(record)}
                </code>
                <span>
                  <small>
                    {record.origin === "human" ? "个人空间" : record.actor}
                  </small>
                  <time>
                    {new Date(record.started_unix_ms).toLocaleString("zh-CN", {
                      month: "2-digit",
                      day: "2-digit",
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                  </time>
                </span>
              </button>
            ))}
          </div>
          {!page?.records.length && (
            <div className="history-empty">
              <Icon name="history" size={28} />
              <h3>
                {listBusy
                  ? "正在读取记录…"
                  : listError
                    ? "记录未加载"
                    : "还没有任务记录"}
              </h3>
              <p>执行命令或传输文件后，可在这里查看结果。</p>
            </div>
          )}
          <footer className="history-pagination">
            <button
              disabled={listBusy || offset === 0}
              onClick={() => {
                setOffset(Math.max(0, offset - 50));
                setShowDetail(false);
              }}
            >
              上一页
            </button>
            <span>第 {Math.floor(offset / 50) + 1} 页</span>
            <button
              disabled={listBusy || !page?.more}
              onClick={() => {
                setOffset(offset + 50);
                setShowDetail(false);
              }}
            >
              下一页
            </button>
          </footer>
        </div>
        <article className="history-detail" aria-busy={detailBusy}>
          <button className="history-back" onClick={() => setShowDetail(false)}>
            <Icon name="back" size={15} />
            返回记录
          </button>
          {metadata ? (
            <>
              <header>
                <div>
                  <h3>{metadata.host_name}</h3>
                  <p>
                    {metadata.space_name} ·{" "}
                    {metadata.origin === "human" ? "个人空间" : metadata.actor}{" "}
                    ·{" "}
                    {new Date(metadata.started_unix_ms).toLocaleString("zh-CN")}
                  </p>
                </div>
                <span className="badge">
                  {STATUS[metadata.status]}
                  {metadata.exit_code !== null
                    ? ` · 退出码 ${metadata.exit_code}`
                    : ""}
                </span>
              </header>
              {metadata.status === "unknown" && (
                <p className="history-warning">
                  {metadata.reason === "client_restarted"
                    ? "客户端重启前操作尚未结束。"
                    : "缺少远端完成证据。"}
                  无法确认远端是否完成，请先检查服务器状态。
                </p>
              )}
              {metadata.status === "running" && (
                <p className="history-warning">
                  {metadata.transfer
                    ? "传输仍在进行，可在文件传输面板查看进度。"
                    : "任务仍在运行，可在会话中查看实时输出。"}
                </p>
              )}
              <pre className="history-command">
                {metadata.transfer ? metadata.transfer.remote : title(metadata)}
              </pre>
              {metadata.transfer?.size != null && (
                <small className="history-note">文件大小 {bytes(metadata.transfer.size)}</small>
              )}
              {metadata.cwd && (
                <small className="history-note">工作目录：{metadata.cwd}</small>
              )}
              {metadata.command_truncated && (
                <small className="history-note">
                  命令超过记录预算，仅保留开头 8 KiB。
                </small>
              )}
              {detailError && (
                <p className="form-error" role="alert">
                  {detailError}
                  <button
                    disabled={detailBusy}
                    onClick={() => setReload((n) => n + 1)}
                  >
                    重新读取详情
                  </button>
                </p>
              )}
              {metadata.deployment?.length ? (
                <ol className="history-deployment">
                  {metadata.deployment.map((step, index) => (
                    <li key={index}>
                      <div>
                        <strong>{step.name}</strong>
                        <small>{step.kind === "put" ? "上传" : "命令"} · {stepLabels[step.status]}{step.exit_code == null ? "" : ` · 退出码 ${step.exit_code}`}</small>
                        {step.reason && !step.job_record && <small>{step.reason}</small>}
                      </div>
                      {step.job_record && <button onClick={() => setSelected(step.job_record!)}>查看任务与日志</button>}
                    </li>
                  ))}
                </ol>
              ) : metadata.transfer ? (
                detail?.stderr.text && <pre className="history-output">{detail.stderr.text}</pre>
              ) : <>
              <div
                className="history-stream-tabs"
                role="tablist"
                aria-label="历史输出流"
              >
                {(["stdout", "stderr"] as const).map((name) => (
                  <button
                    key={name}
                    role="tab"
                    aria-selected={stream === name}
                    tabIndex={stream === name ? 0 : -1}
                    id={`history-${name}-tab`}
                    aria-controls="history-output"
                    onClick={() => setStream(name)}
                    onKeyDown={(e) => {
                      if (
                        ["ArrowLeft", "ArrowRight", "Home", "End"].includes(
                          e.key,
                        )
                      ) {
                        e.preventDefault();
                        const next =
                          e.key === "Home"
                            ? "stdout"
                            : e.key === "End"
                              ? "stderr"
                              : name === "stdout"
                                ? "stderr"
                                : "stdout";
                        setStream(next);
                        document.getElementById(`history-${next}-tab`)?.focus();
                      }
                    }}
                  >
                    {name === "stdout"
                      ? metadata.pty
                        ? "终端合并输出"
                        : "标准输出"
                      : "错误输出"}
                    {detail
                      ? ` · ${detail[name].total_bytes.toLocaleString()} B`
                      : ""}
                  </button>
                ))}
              </div>
              {metadata.archive && (
                <button className="history-log-toggle" aria-pressed={archiveRecord === metadata.record}
                  onClick={() => setArchiveRecord((old) => old === metadata.record ? undefined : metadata.record)}>
                  {archiveRecord === metadata.record ? "返回输出尾部" : "查看归档日志"}
                </button>
              )}
              {metadata.archive?.issue && archiveRecord !== metadata.record && <p className="history-note">{metadata.archive.issue}</p>}
              {archiveRecord === metadata.record ? (
                <ArchivedLog key={`${metadata.record}:${stream}`} record={metadata.record} stream={stream} />
              ) : <>
              {output?.truncated && (
                <p className="history-note">
                  较早内容已超出保留预算，以下为输出尾部。
                </p>
              )}
              {output?.invalid_utf8 && (
                <p className="history-note">
                  含无效 UTF-8 字节，使用替代字符显示。
                </p>
              )}
              <pre
                id="history-output"
                className="history-output"
                role="tabpanel"
                aria-labelledby={`history-${stream}-tab`}
                tabIndex={0}
              >
                {output?.text ||
                  (detailBusy
                    ? "正在读取输出…"
                    : detailError
                      ? "输出未加载"
                      : "此路没有保留的输出")}
              </pre>
              </>}
              </>}
            </>
          ) : (
            <div className="history-empty">
              <Icon name="history" size={30} />
              <h3>选择一条记录</h3>
              <p>查看服务器、操作结果及保留的输出。</p>
            </div>
          )}
        </article>
      </div>
    </section>
  );
}
