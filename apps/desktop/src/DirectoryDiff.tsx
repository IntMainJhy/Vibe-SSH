import { useEffect, useRef, useState } from "react";
import { call, explain } from "./api";
import { Modal } from "./Modal";
import { Icon } from "./Icon";
import { bytes } from "./Transfers";

type Entry = {
  path: string;
  kind: string;
  size: number | null;
  excluded: boolean;
};
type Manifest = { entries: Entry[]; excludes: string[]; truncated: boolean };
type Source = { id: string; name: string; manifest: Manifest };
type Review = { revision: string; paths: { path: string; replaces: boolean }[] };
type ApplyResult = {
  applied: { path: string; backup: string | null }[];
  stopped: { path: string; code: string; message: string } | null;
};
type Preview = {
  remote: string;
  truncated: boolean;
  apply?: Review;
  entries: {
    path: string;
    local_size: number | null;
    remote_size: number | null;
    status: string;
  }[];
};
const labels: Record<string, string> = {
  same: "内容相同",
  changed: "内容已修改",
  local_only: "仅项目目录",
  remote_only: "仅服务器",
  type_conflict: "类型不同",
  directory: "目录",
  unsupported: "链接 / 特殊文件",
  excluded: "已忽略",
  unverified: "未校验",
};

export function DirectoryDiff({
  server,
  space,
  remote,
  writable,
  onApplied,
  onClose,
}: {
  server: string;
  space: string;
  remote: string;
  writable: boolean;
  onApplied: () => void;
  onClose: () => void;
}) {
  const [source, setSource] = useState<Source>();
  const [selected, setSelected] = useState<string[]>([]);
  const [rules, setRules] = useState("");
  const [applied, setApplied] = useState("");
  const [preview, setPreview] = useState<Preview>();
  const [compared, setCompared] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [result, setResult] = useState<ApplyResult>();
  const [busy, setBusy] = useState(false);
  const applying = useRef(false);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const id = useRef<string | undefined>(undefined);
  useEffect(
    () => () => {
      if (id.current)
        void call("forget_diff", { id: id.current }).catch(() => {});
    },
    [],
  );
  const extras = () =>
    rules
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean);
  const invalidate = () => {
    setCompared(false);
    setConfirm(false);
    setPreview((old) =>
      old
        ? {
            ...old,
            apply: undefined,
            entries: old.entries.map((e) =>
              ["same", "changed"].includes(e.status)
                ? { ...e, status: "unverified" }
                : e,
            ),
          }
        : old,
    );
  };
  const choose = async () => {
    setBusy(true);
    setError("");
    try {
      const next = await call<Source | null>("choose_diff", {
        excludes: extras(),
      });
      if (next) {
        if (id.current)
          void call("forget_diff", { id: id.current }).catch(() => {});
        id.current = next.id;
        setSource(next);
        setApplied(rules);
        setPreview(undefined);
        setResult(undefined);
        setSelected([]);
        setCompared(false);
        setPreview(
          await call<Preview>("compare_directory", {
            id: next.id,
            selected: [],
            space,
            remote,
          }),
        );
      }
    } catch (e) {
      setError(explain(e));
    } finally {
      setBusy(false);
    }
  };
  const rescan = async () => {
    if (!source) return;
    setBusy(true);
    setError("");
    setPreview(undefined);
    setSelected([]);
    setCompared(false);
    try {
      const manifest = await call<Manifest>("rescan_diff", {
        id: source.id,
        excludes: extras(),
      });
      setSource({ ...source, manifest });
      setApplied(rules);
      setPreview(
        await call<Preview>("compare_directory", {
          id: source.id,
          selected: [],
          space,
          remote,
        }),
      );
    } catch (e) {
      setError(explain(e));
    } finally {
      setBusy(false);
    }
  };
  const compare = async () => {
    if (!source) return;
    invalidate();
    setBusy(true);
    setError("");
    try {
      setPreview(
        await call<Preview>("compare_directory", {
          id: source.id,
          selected,
          space,
          remote,
        }),
      );
      setCompared(true);
    } catch (e) {
      setError(explain(e));
    } finally {
      setBusy(false);
    }
  };
  const changedRules = rules !== applied;
  const review = compared && writable && !changedRules ? preview?.apply : undefined;
  const apply = async () => {
    if (!source || !review || !confirm || busy || applying.current) return;
    applying.current = true;
    invalidate();
    setResult(undefined);
    setError("");
    setBusy(true);
    try {
      const receipt = await call<ApplyResult>("apply_directory", {
        id: source.id,
        revision: review.revision,
        space,
      });
      setPreview(undefined);
      setResult(receipt);
      onApplied();
    } catch (e) {
      setError(explain(e));
    } finally {
      applying.current = false;
      setBusy(false);
    }
  };
  const selectable =
    source?.manifest.entries
      .filter(
        (e) =>
          e.kind === "file" &&
          !e.excluded &&
          e.size !== null &&
          e.size <= 8_388_608,
      )
      .map((e) => e.path) ?? [];
  const rows =
    preview?.entries ??
    source?.manifest.entries.map((e) => ({
      path: e.path,
      local_size: e.size,
      remote_size: null,
      status: e.excluded
        ? "excluded"
        : e.kind === "directory"
          ? "directory"
          : e.kind === "file"
            ? "unverified"
            : "unsupported",
    })) ??
    [];
  return (
    <Modal
      titleId="diff-title"
      busy={busy}
      onClose={onClose}
      className="directory-diff-dialog"
    >
      <header className="diff-heading">
        <div>
          <h2 id="diff-title">比较目录</h2>
          <p>{server} · {writable ? "预览并应用" : "只读预览"}</p>
        </div>
        <button
          className="icon-button"
          aria-label="关闭目录比较"
          disabled={busy}
          onClick={onClose}
        >
          <Icon name="close" />
        </button>
      </header>
      <div className="diff-body">
        <div className="diff-targets">
          <div>
            <span>项目目录</span>
            <strong title={source?.name}>{source?.name ?? "尚未选择"}</strong>
            <button disabled={busy} onClick={() => void choose()}>
              <Icon name="folder" size={15} />
              {source ? "更换目录" : "选择项目目录"}
            </button>
          </div>
          <div>
            <span>服务器目录</span>
            <strong title={remote}>{remote}</strong>
            <small>比较只读取所选文件，应用前需确认清单。</small>
          </div>
        </div>
        <details className="diff-rules">
          <summary>
            忽略规则
            {source
              ? ` · ${source.manifest.entries.filter((e) => e.excluded).length} 项已忽略`
              : ""}
          </summary>
          <p>
            默认忽略 .git、依赖、构建产物及 .env*、私钥。每行添加一条单路径分量
            glob，不含 /。
          </p>
          <textarea
            aria-label="附加忽略规则"
            rows={2}
            placeholder={"例如：*.log\ncache"}
            value={rules}
            disabled={busy}
            onChange={(e) => {
              setRules(e.target.value);
              invalidate();
              setPreview(undefined);
              setSelected([]);
            }}
          />
          {source && (
            <>
              <p className="diff-rule-list">
                生效规则：{source.manifest.excludes.join("、")}
              </p>
              <button disabled={busy} onClick={() => void rescan()}>
                应用规则并重新列出
              </button>
            </>
          )}
        </details>
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        {result && (
          <section className="diff-receipt" aria-label="目录应用结果">
            <p role="status">已应用 {result.applied.length} 个文件</p>
            <ul>
              {result.applied.map(({ path, backup }) => (
                <li key={path}>
                  <code>{path}</code>
                  {backup && <small>服务器备份 <code>{backup}</code></small>}
                </li>
              ))}
            </ul>
            {result.stopped && (
              <p className="form-error" role="alert">
                在 <code>{result.stopped.path}</code> 停止。{result.stopped.message}
              </p>
            )}
          </section>
        )}
        {source ? (
          <>
            <div className="diff-list-heading">
              <button disabled={busy} onClick={() => void rescan()}>
                重新列出两端
              </button>
              <label className="check-label">
                <input
                  type="checkbox"
                  checked={
                    selectable.length > 0 &&
                    selectable.every((p) => selected.includes(p))
                  }
                  disabled={busy || changedRules || !selectable.length}
                  onChange={(e) => {
                    setSelected(e.target.checked ? selectable : []);
                    invalidate();
                  }}
                />
                选择可比较文件
              </label>
              <input
                aria-label="筛选比较路径"
                placeholder="筛选相对路径…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
            <p className="diff-stage" role="status">
              {applying.current
                ? "正在核对并应用所选更改…"
                : busy
                ? "正在列出目录或校验所选文件…"
                : changedRules
                  ? "规则有改动，请应用后重新选择文件。"
                  : compared
                    ? "所选文件比较完成。未校验的文件仍需检查。"
                    : "仅列出元数据，尚未比较内容。勾选的普通文件才会读取内容。"}
            </p>
            <div className="diff-table-wrap">
              <table className="file-table diff-table">
                <thead>
                  <tr>
                    <th>
                      <span className="sr-only">选择</span>
                    </th>
                    <th>相对路径</th>
                    <th>项目</th>
                    <th>服务器</th>
                    <th>状态</th>
                  </tr>
                </thead>
                <tbody>
                  {rows
                    .filter((e) =>
                      e.path
                        .toLocaleLowerCase()
                        .includes(query.toLocaleLowerCase()),
                    )
                    .map((e) => (
                      <tr key={e.path}>
                        <td>
                          <input
                            type="checkbox"
                            aria-label={`比较 ${e.path}`}
                            checked={selected.includes(e.path)}
                            disabled={
                              busy ||
                              changedRules ||
                              !selectable.includes(e.path)
                            }
                            onChange={(event) => {
                              setSelected((old) =>
                                event.target.checked
                                  ? [...old, e.path]
                                  : old.filter((p) => p !== e.path),
                              );
                              invalidate();
                            }}
                          />
                        </td>
                        <td>
                          <code title={e.path}>{e.path}</code>
                          <small className="diff-mobile-sizes">
                            项目{" "}
                            {e.local_size === null ? "—" : bytes(e.local_size)}{" "}
                            · 服务器{" "}
                            {e.remote_size === null
                              ? "—"
                              : bytes(e.remote_size)}
                          </small>
                        </td>
                        <td>
                          {e.local_size === null ? "—" : bytes(e.local_size)}
                        </td>
                        <td>
                          {e.remote_size === null ? "—" : bytes(e.remote_size)}
                        </td>
                        <td>
                          <span className="diff-status" data-status={e.status}>
                            {labels[e.status] ?? e.status}
                          </span>
                        </td>
                      </tr>
                    ))}
                </tbody>
              </table>
              {!rows.some((e) =>
                e.path.toLocaleLowerCase().includes(query.toLocaleLowerCase()),
              ) && (
                <p className="diff-stage">
                  {query ? "没有匹配的相对路径。" : "此目录没有可比较的项目。"}
                </p>
              )}
            </div>
            {(source.manifest.truncated || preview?.truncated) && (
              <p className="form-error">
                目录超出预览上限，只列出部分项目；缺失项不能判定为仅在某一端存在。请选择更小的目录。
              </p>
            )}
          </>
        ) : (
          <div className="diff-empty">
            <Icon name="folder" size={32} />
            <h3>先选择项目目录</h3>
            <p>先查看文件与忽略范围，再选择要校验的内容。</p>
          </div>
        )}
      </div>
      <footer className="diff-footer">
        <div>
          <strong>已选 {selected.length} 个文件</strong>
          <small>每文件最多 8 MiB，总计 32 MiB；超限保留为未校验。</small>
        </div>
        <button disabled={busy} onClick={onClose}>
          关闭
        </button>
        <button
          className={review ? undefined : "primary"}
          disabled={busy || !source || !selected.length || changedRules}
          onClick={() => void compare()}
        >
          比较所选文件
        </button>
        {review && (
          <button className="primary" disabled={busy} onClick={() => setConfirm(true)}>
            检查并应用 {review.paths.length} 项
          </button>
        )}
      </footer>
      {confirm && review && (
        <Modal titleId="diff-confirm-title" onClose={() => setConfirm(false)}>
          <h2 id="diff-confirm-title">确认应用目录更改</h2>
          <p>目标目录 <code>{preview?.remote}</code></p>
          <p>将逐个应用下列文件，替换时会在服务器上保留原文件的备份。</p>
          <ul className="diff-review-list">
            {review.paths.map(({ path, replaces }) => (
              <li key={path}>
                <code>{path}</code>
                <small>{replaces ? "替换并备份" : "新增"}</small>
              </li>
            ))}
          </ul>
          <div className="form-actions">
            <button autoFocus onClick={() => setConfirm(false)}>返回比较</button>
            <button className="primary" onClick={() => void apply()}>确认应用</button>
          </div>
        </Modal>
      )}
    </Modal>
  );
}
