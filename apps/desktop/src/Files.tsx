import { useEffect, useRef, useState } from "react";
import { call, explain } from "./api";
import { Icon } from "./Icon";
import { Modal } from "./Modal";
import { TextEditor } from "./TextEditor";
import { DirectoryDiff } from "./DirectoryDiff";
import { bytes, type TransferStart } from "./Transfers";

type Entry = {
  name: string;
  path: string;
  kind: "file" | "directory" | "symlink" | "other";
  size: number | null;
  modified: number | null;
  permissions: number | null;
};
type Directory = { path: string; entries: Entry[]; truncated: boolean };
type Action = { kind: "mkdir" | "rename" | "delete"; entry?: Entry };
export function Files({
  server,
  space,
  writable,
  transfer,
  revision,
  initialPath,
  onDirectory,
}: {
  server: string;
  space: string;
  writable: boolean;
  transfer: TransferStart;
  revision: number;
  initialPath: string;
  onDirectory: (path: string) => void;
}) {
  const [path, setPath] = useState(initialPath);
  const [location, setLocation] = useState(initialPath);
  const [data, setData] = useState<Directory>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [hidden, setHidden] = useState(false);
  const [query, setQuery] = useState("");
  const [reload, setReload] = useState(0);
  const [editing, setEditing] = useState<Entry>();
  const [comparing, setComparing] = useState(false);
  const [action, setAction] = useState<Action>();
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);
  const [actionError, setActionError] = useState("");
  const serial = useRef(0);
  const restoreEditor = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (editing || loading || !restoreEditor.current) return;
    const button = Array.from(
      document.querySelectorAll<HTMLButtonElement>("button[data-preview-path]"),
    ).find((button) => button.dataset.previewPath === restoreEditor.current);
    if (button) {
      button.focus();
      restoreEditor.current = undefined;
    }
  }, [editing, loading, data]);
  const directoryChanged = useRef(onDirectory);
  directoryChanged.current = onDirectory;
  useEffect(() => {
    const generation = ++serial.current;
    setLoading(true);
    setData(undefined);
    setError("");
    void call<Directory>("files", { space, path })
      .then((result) => {
        if (serial.current !== generation) return;
        setData(result);
        directoryChanged.current(result.path);
        setLocation(result.path);
      })
      .catch((e) => {
        if (serial.current === generation) setError(explain(e));
      })
      .finally(() => {
        if (serial.current === generation) setLoading(false);
      });
    return () => {
      serial.current++;
    };
  }, [space, path, reload, revision]);
  const open = (next: string) => {
    setPath(next);
    setReload((n) => n + 1);
  };
  const prepare = (next: Action) => {
    setAction(next);
    setName(next.entry?.name ?? "");
    setActionError("");
  };
  const validName =
    !!name.trim() &&
    name !== "." &&
    name !== ".." &&
    !name.includes("/") &&
    !name.includes("\0");
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!action || saving || !writable || !data) return;
    setSaving(true);
    setActionError("");
    const destination = `${data.path.replace(/\/$/, "")}/${name}`;
    try {
      await call("file_action", {
        space,
        action: action.kind,
        path: action.kind === "mkdir" ? destination : action.entry!.path,
        destination: action.kind === "rename" ? destination : null,
      });
      setAction(undefined);
      setReload((n) => n + 1);
    } catch (e) {
      setActionError(explain(e));
    } finally {
      setSaving(false);
    }
  };
  const entries =
    data?.entries.filter(
      (entry) =>
        (hidden || !entry.name.startsWith(".")) &&
        entry.name.toLocaleLowerCase().includes(query.toLocaleLowerCase()),
    ) ?? [];
  return (
    <section className="files-page">
      <div className="file-toolbar">
        <button
          className="icon-button"
          aria-label="上级目录"
          disabled={loading || !data || data.path === "/"}
          onClick={() =>
            open(data!.path.split("/").slice(0, -1).join("/") || "/")
          }
        >
          <Icon name="back" size={17} />
        </button>
        <button
          className="icon-button"
          aria-label="主目录"
          disabled={loading}
          onClick={() => open(".")}
        >
          <Icon name="home" size={17} />
        </button>
        <form
          className="file-location"
          onSubmit={(event) => {
            event.preventDefault();
            open(location);
          }}
        >
          <Icon name="folder" size={16} />
          <input
            aria-label="远端目录路径"
            value={location}
            onChange={(e) => setLocation(e.target.value)}
          />
          <button type="submit" disabled={loading}>
            前往
          </button>
        </form>
        <button
          className="icon-button"
          aria-label="刷新文件"
          disabled={loading}
          onClick={() => setReload((n) => n + 1)}
        >
          <Icon name="refresh" size={16} />
        </button>
      </div>
      <div className="file-options">
        <label className="file-filter">
          <Icon name="search" size={15} />
          <input
            placeholder="筛选文件"
            aria-label="筛选文件"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </label>
        <label className="check-label">
          <input
            type="checkbox"
            checked={hidden}
            onChange={(e) => setHidden(e.target.checked)}
          />
          隐藏文件
        </label>
        <span className="grow" />
        <button disabled={!data || loading} onClick={() => setComparing(true)}>
          比较目录
        </button>
        <button
          disabled={!writable || !data || loading}
          onClick={() => prepare({ kind: "mkdir" })}
        >
          <Icon name="plus" size={15} />
          新建文件夹
        </button>
        <button
          className="primary"
          disabled={!writable || !data || loading}
          onClick={() => void transfer("upload", space, data!.path)}
        >
          <Icon name="upload" size={15} />
          上传文件
        </button>
      </div>
      {!writable && (
        <p className="file-readonly">
          <Icon name="shield" size={14} />
          当前为只读观察；接管空间后可上传和管理文件。
        </p>
      )}
      {error && (
        <div className="error-banner" role="alert">
          <span>{error}</span>
          <button onClick={() => setReload((n) => n + 1)}>重试</button>
        </div>
      )}
      <div className="file-table-wrap" aria-busy={loading}>
        <table className="file-table">
          <thead>
            <tr>
              <th>名称</th>
              <th>大小</th>
              <th>修改时间</th>
              <th>权限</th>
              <th>
                <span className="sr-only">操作</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {entries.map((entry) => (
              <tr key={entry.path}>
                <td>
                  <button
                    className="file-name"
                    title={entry.path}
                    disabled={loading || entry.kind === "other"}
                    onClick={() =>
                      entry.kind === "file"
                        ? setEditing(entry)
                        : open(entry.path)
                    }
                  >
                    <Icon
                      name={
                        entry.kind === "directory"
                          ? "folder"
                          : entry.kind === "symlink"
                            ? "link"
                            : "file"
                      }
                      size={18}
                    />
                    <span>{entry.name}</span>
                    {entry.kind === "symlink" && <small>链接</small>}
                  </button>
                </td>
                <td>{entry.kind === "directory" ? "—" : bytes(entry.size)}</td>
                <td>
                  {entry.modified
                    ? new Date(entry.modified * 1000).toLocaleString("zh-CN", {
                        month: "2-digit",
                        day: "2-digit",
                        hour: "2-digit",
                        minute: "2-digit",
                      })
                    : "—"}
                </td>
                <td>
                  <code>
                    {entry.permissions == null
                      ? "—"
                      : (entry.permissions & 0o7777).toString(8)}
                  </code>
                </td>
                <td>
                  <div className="file-row-actions">
                    {entry.kind === "file" && (
                      <button
                        className="icon-button"
                        data-preview-path={entry.path}
                        aria-label={`查看 / 编辑 ${entry.name}`}
                        title="查看 / 编辑文本"
                        disabled={loading}
                        onClick={() => setEditing(entry)}
                      >
                        <Icon name="edit" size={15} />
                      </button>
                    )}
                    {entry.kind === "file" && (
                      <button
                        className="icon-button"
                        aria-label={`下载 ${entry.name}`}
                        title="下载"
                        disabled={loading}
                        onClick={() =>
                          void transfer("download", space, entry.path)
                        }
                      >
                        <Icon name="download" size={15} />
                      </button>
                    )}
                    <button
                      className="icon-button"
                      aria-label={`重命名 ${entry.name}`}
                      title="重命名"
                      disabled={!writable || loading}
                      onClick={() => prepare({ kind: "rename", entry })}
                    >
                      <Icon name="rename" size={15} />
                    </button>
                    <button
                      className="icon-button"
                      aria-label={`删除 ${entry.name}`}
                      title="删除"
                      disabled={!writable || loading}
                      onClick={() => prepare({ kind: "delete", entry })}
                    >
                      <Icon name="trash" size={15} />
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {(!data || !entries.length) && (
          <div className="file-empty">
            <Icon name="folder" size={30} />
            <p>
              {loading
                ? "正在读取目录…"
                : error
                  ? "目录未加载"
                  : query
                    ? "没有匹配的文件"
                    : "这个目录没有可显示的文件"}
            </p>
          </div>
        )}
      </div>
      <footer className="file-footer">
        <span>
          {loading ? "读取中…" : `${entries.length} 个项目`}
          {data?.truncated && " · 目录超过显示上限，请进入子目录查看"}
        </span>
        <span>SFTP · {writable ? "可管理" : "只读"}</span>
      </footer>
      {editing && (
        <TextEditor
          server={server}
          space={space}
          path={editing.path}
          writable={writable}
          onClose={() => {
            restoreEditor.current = editing.path;
            setEditing(undefined);
          }}
          onSaved={() => setReload((n) => n + 1)}
        />
      )}
      {comparing && data && (
        <DirectoryDiff
          server={server}
          space={space}
          remote={data.path}
          onClose={() => setComparing(false)}
        />
      )}
      {action && (
        <Modal
          titleId="file-action-title"
          busy={saving}
          onClose={() => setAction(undefined)}
        >
          <form onSubmit={(event) => void submit(event)}>
            <h2 id="file-action-title">
              {action.kind === "mkdir"
                ? "新建文件夹"
                : action.kind === "rename"
                  ? "重命名"
                  : "删除文件或空文件夹？"}
            </h2>
            {action.kind === "delete" ? (
              <p className="file-delete-path">
                {action.entry!.path}
                <br />
                此操作无法撤销；非空文件夹不会被递归删除。
              </p>
            ) : (
              <label className="file-edit-label">
                名称
                <input
                  autoFocus
                  aria-label="文件名称"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  disabled={saving}
                />
                <small>已有同名项目不会被覆盖。</small>
              </label>
            )}
            {actionError && (
              <p className="form-error" role="alert">
                {actionError}
              </p>
            )}
            <div className="actions">
              <button
                type="button"
                disabled={saving}
                onClick={() => setAction(undefined)}
              >
                取消
              </button>
              <button
                className={action.kind === "delete" ? "danger" : "primary"}
                type="submit"
                disabled={
                  saving ||
                  !writable ||
                  (action.kind !== "delete" && !validName)
                }
              >
                {saving
                  ? "处理中…"
                  : action.kind === "delete"
                    ? "确认删除"
                    : "保存"}
              </button>
            </div>
          </form>
        </Modal>
      )}
    </section>
  );
}
