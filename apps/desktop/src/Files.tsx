import { useEffect, useRef, useState, type ReactNode } from "react";
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
const isDirectory = (entry: Entry) =>
  entry.kind === "directory" || entry.kind === "symlink";
const modified = (entry: Entry) =>
  entry.modified
    ? new Date(entry.modified * 1000).toLocaleString("zh-CN", {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
      })
    : "—";
const permissions = (entry: Entry) =>
  entry.permissions == null ? "—" : (entry.permissions & 0o7777).toString(8);
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
  const [selected, setSelected] = useState<Entry>();
  const [children, setChildren] = useState<Directory>();
  const [childLoading, setChildLoading] = useState(false);
  const [childError, setChildError] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [hidden, setHidden] = useState(false);
  const [query, setQuery] = useState("");
  const [reload, setReload] = useState(0);
  const [editing, setEditing] = useState<Entry>();
  const [comparing, setComparing] = useState<string>();
  const [action, setAction] = useState<Action>();
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);
  const [actionError, setActionError] = useState("");
  const serial = useRef(0);
  const childSerial = useRef(0);
  const prefetched = useRef<Directory | undefined>(undefined);
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
  }, [editing, loading, data, selected]);
  const directoryChanged = useRef(onDirectory);
  directoryChanged.current = onDirectory;
  useEffect(() => {
    const generation = ++serial.current;
    const cached = prefetched.current;
    prefetched.current = undefined;
    setLoading(!cached);
    setError("");
    const request =
      cached?.path === path
        ? Promise.resolve(cached)
        : call<Directory>("files", { space, path });
    void request
      .then((result) => {
        if (serial.current !== generation) return;
        setData(result);
        setSelected((current) =>
          result.entries.find((entry) => entry.path === current?.path),
        );
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
  useEffect(() => {
    const generation = ++childSerial.current;
    setChildren(undefined);
    setChildError("");
    if (!selected || !isDirectory(selected)) {
      setChildLoading(false);
      return;
    }
    setChildLoading(true);
    void call<Directory>("files", { space, path: selected.path })
      .then((result) => {
        if (childSerial.current === generation) setChildren(result);
      })
      .catch((e) => {
        if (childSerial.current === generation) setChildError(explain(e));
      })
      .finally(() => {
        if (childSerial.current === generation) setChildLoading(false);
      });
    return () => {
      childSerial.current++;
    };
  }, [space, selected?.path, selected?.kind, reload, revision]);
  const directorySelected = selected && isDirectory(selected);
  const activeDirectory = directorySelected ? children : data;
  const busy = loading || (!!directorySelected && childLoading);
  useEffect(() => {
    if (!activeDirectory) return;
    directoryChanged.current(activeDirectory.path);
    setLocation(activeDirectory.path);
  }, [activeDirectory]);
  const open = (next: string) => {
    setPath(next);
    setReload((n) => n + 1);
  };
  const choose = (entry: Entry, fromChild: boolean) => {
    if (!fromChild && selected?.path === entry.path) return;
    childSerial.current++;
    if (fromChild && children) {
      prefetched.current = children;
      setData(children);
      setPath(children.path);
    }
    setChildren(undefined);
    setSelected(entry);
    setLocation(isDirectory(entry) ? entry.path : (fromChild ? children : data)!.path);
    setQuery("");
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
    if (!action || saving || !writable || (!activeDirectory && !action.entry)) return;
    setSaving(true);
    setActionError("");
    const parent = action.entry
      ? action.entry.path.split("/").slice(0, -1).join("/")
      : activeDirectory!.path;
    const destination = `${parent.replace(/\/$/, "")}/${name}`;
    try {
      await call("file_action", {
        space,
        action: action.kind,
        path: action.kind === "mkdir" ? destination : action.entry!.path,
        destination: action.kind === "rename" ? destination : null,
      });
      setAction(undefined);
      if (action.entry?.path === selected?.path) setSelected(undefined);
      setReload((n) => n + 1);
    } catch (e) {
      setActionError(explain(e));
    } finally {
      setSaving(false);
    }
  };
  const filter = (directory?: Directory) =>
    directory?.entries.filter(
      (entry) =>
        (hidden || !entry.name.startsWith(".")) &&
        entry.name.toLocaleLowerCase().includes(query.toLocaleLowerCase()),
    ) ?? [];
  const entries = filter(data);
  const childEntries = filter(children);
  return (
    <section className="files-page">
      <div className="file-toolbar">
        <button
          className="icon-button"
          aria-label="上级目录"
          disabled={loading || !data || data.path === "/" && !directorySelected}
          onClick={() => directorySelected
            ? setSelected(undefined)
            : open(data!.path.split("/").slice(0, -1).join("/") || "/")}
        >
          <Icon name="back" size={17} />
        </button>
        <button
          className="icon-button"
          aria-label="主目录"
          disabled={busy}
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
          disabled={busy}
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
        <button
          disabled={!activeDirectory || busy}
          onClick={() => setComparing(activeDirectory!.path)}
        >
          比较目录
        </button>
        <button
          disabled={!writable || !activeDirectory || busy}
          onClick={() => prepare({ kind: "mkdir" })}
        >
          <Icon name="plus" size={15} />
          新建文件夹
        </button>
        <button
          className="primary"
          disabled={!writable || !activeDirectory || busy}
          onClick={() => void transfer("upload", space, activeDirectory!.path)}
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
      <div className="file-columns" aria-label="远端文件分栏浏览">
        <FileColumn
          title="上级目录"
          directory={data}
          entries={entries}
          loading={loading}
          error={error}
          query={query}
          selected={selected?.path}
          onChoose={(entry) => choose(entry, false)}
        />
        {directorySelected ? (
          <FileColumn
            title="下级目录"
            directory={children}
            path={selected.path}
            entries={childEntries}
            loading={childLoading}
            disabled={loading}
            error={childError}
            query={query}
            onChoose={(entry) => choose(entry, true)}
            onRetry={() => setReload((n) => n + 1)}
            actions={
              <>
                <button className="icon-button" aria-label={`重命名 ${selected.name}`}
                  title="重命名" disabled={!writable || loading}
                  onClick={() => prepare({ kind: "rename", entry: selected })}>
                  <Icon name="rename" size={15} />
                </button>
                <button className="icon-button" aria-label={`删除 ${selected.name}`}
                  title="删除" disabled={!writable || loading}
                  onClick={() => prepare({ kind: "delete", entry: selected })}>
                  <Icon name="trash" size={15} />
                </button>
              </>
            }
          />
        ) : (
          <section className="file-column file-inspector" aria-label="文件信息">
            <header className="file-column-heading">
              <div><span>文件信息</span><strong>{selected?.name ?? "选择一个项目"}</strong></div>
            </header>
            {selected ? (
              <div className="file-details">
                <div className="file-detail-icon"><Icon name="file" size={32} /></div>
                <h3>{selected.name}</h3>
                <p className="file-detail-path">{selected.path}</p>
                <dl>
                  <div><dt>类型</dt><dd>{selected.kind === "file" ? "文件" : "特殊文件"}</dd></div>
                  <div><dt>大小</dt><dd>{bytes(selected.size)}</dd></div>
                  <div><dt>修改时间</dt><dd>{modified(selected)}</dd></div>
                  <div><dt>权限</dt><dd><code>{permissions(selected)}</code></dd></div>
                </dl>
                <div className="file-detail-actions">
                  {selected.kind === "file" && <>
                    <button className="primary" data-preview-path={selected.path}
                      disabled={loading} onClick={() => setEditing(selected)}>
                      <Icon name="edit" size={15} />查看 / 编辑
                    </button>
                    <button disabled={loading}
                      onClick={() => void transfer("download", space, selected.path)}>
                      <Icon name="download" size={15} />下载
                    </button>
                  </>}
                  <button disabled={!writable || loading}
                    onClick={() => prepare({ kind: "rename", entry: selected })}>
                    <Icon name="rename" size={15} />重命名
                  </button>
                  <button disabled={!writable || loading}
                    onClick={() => prepare({ kind: "delete", entry: selected })}>
                    <Icon name="trash" size={15} />删除
                  </button>
                </div>
              </div>
            ) : (
              <div className="file-empty file-column-placeholder">
                <Icon name="folder" size={32} />
                <p>选择文件夹，在这里查看下级内容</p>
                <small>选择文件可查看信息、编辑或下载</small>
              </div>
            )}
          </section>
        )}
      </div>
      <footer className="file-footer">
        <span className="file-operation-path" title={activeDirectory?.path}>
          操作目录 <code>{activeDirectory?.path ?? "—"}</code>
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
      {comparing && (
        <DirectoryDiff
          server={server}
          space={space}
          remote={comparing}
          onClose={() => setComparing(undefined)}
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

function FileColumn({
  title, directory, path, entries, selected, loading, disabled = false,
  error, query, onChoose, onRetry, actions,
}: {
  title: string;
  directory?: Directory;
  path?: string;
  entries: Entry[];
  selected?: string;
  loading: boolean;
  disabled?: boolean;
  error: string;
  query: string;
  onChoose: (entry: Entry) => void;
  onRetry?: () => void;
  actions?: ReactNode;
}) {
  return (
    <section className="file-column" aria-label={title} aria-busy={loading}>
      <header className="file-column-heading">
        <div>
          <span>{title}</span>
          <strong title={directory?.path ?? path}>{directory?.path ?? path ?? "—"}</strong>
        </div>
        <div className="file-column-actions">{actions}</div>
      </header>
      <div className="file-column-scroll">
        {error && onRetry && (
          <div className="file-column-error" role="alert">
            <Icon name="alert" size={22} />
            <p>{error}</p>
            <button onClick={onRetry}>重试</button>
          </div>
        )}
        <ul className="file-column-list">
          {entries.map((entry) => (
            <li key={entry.path}>
              <button className="file-column-entry" title={entry.path}
                aria-label={`选择 ${entry.name}`} aria-pressed={selected === entry.path}
                data-preview-path={entry.path}
                disabled={loading || disabled} onClick={() => onChoose(entry)}>
                <Icon name={entry.kind === "directory" ? "folder" : entry.kind === "symlink" ? "link" : "file"} size={19} />
                <span className="file-entry-copy">
                  <strong>{entry.name}</strong>
                  <small>{entry.kind === "directory" ? "文件夹" : entry.kind === "symlink" ? "符号链接" : bytes(entry.size)} · {modified(entry)} · {permissions(entry)}</small>
                </span>
                {isDirectory(entry) && <Icon name="chevron" size={14} />}
              </button>
            </li>
          ))}
        </ul>
        {!entries.length && !error && (
          <div className="file-empty">
            <Icon name="folder" size={30} />
            <p>{loading ? "正在读取目录…" : query ? "没有匹配的文件" : "这个目录没有可显示的文件"}</p>
          </div>
        )}
      </div>
      <div className="file-column-status" role="status">
        {loading ? "读取中…" : `${entries.length} 个项目`}
        {directory?.truncated && " · 已达显示上限"}
      </div>
    </section>
  );
}
