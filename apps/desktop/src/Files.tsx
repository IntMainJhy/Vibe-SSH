import { useEffect, useMemo, useRef, useState } from "react";
import { call, explain } from "./api";
import { Icon } from "./Icon";
import { Modal } from "./Modal";
import { TextEditor } from "./TextEditor";
import { DirectoryDiff } from "./DirectoryDiff";
import { bytes, type TransferStart } from "./Transfers";
import { arrangeFiles, fileModified, filePage, type Entry, type FileSort } from "./file-list";

type Directory = { path: string; entries: Entry[]; truncated: boolean };
type Action = { kind: "mkdir" | "rename" | "delete" | "chmod"; entry?: Entry };
const isDirectory = (entry: Entry) =>
  entry.kind === "directory" || entry.kind === "symlink";
const modified = (entry: Entry) => fileModified(entry.modified);
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
  const requestedPath = useRef(initialPath);
  const displayedPath = useRef<string | undefined>(undefined);
  const [data, setData] = useState<Directory>();
  const [home, setHome] = useState<string>();
  const [location, setLocation] = useState<string>();
  const [selected, setSelected] = useState<string>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<{ path: string; message: string }>();
  const [hidden, setHidden] = useState(false);
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<FileSort>("name-asc");
  const [reload, setReload] = useState(0);
  const [editing, setEditing] = useState<Entry>();
  const [comparing, setComparing] = useState<string>();
  const [action, setAction] = useState<Action>();
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);
  const [actionError, setActionError] = useState("");
  const list = useRef<HTMLDivElement>(null);
  const breadcrumbs = useRef<HTMLElement>(null);
  const restoreEditor = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (editing || loading || !restoreEditor.current) return;
    const button = Array.from(
      list.current?.querySelectorAll<HTMLButtonElement>("button[data-preview-path]") ?? [],
    ).find((button) => button.dataset.previewPath === restoreEditor.current);
    button?.focus();
    restoreEditor.current = undefined;
  }, [editing, loading, data]);
  const directoryChanged = useRef(onDirectory);
  directoryChanged.current = onDirectory;
  useEffect(() => {
    let active = true;
    const path = requestedPath.current;
    setLoading(true);
    setError(undefined);
    void call<Directory>("files", { space, path })
      .then((result) => {
        if (!active) return;
        if (path === ".") setHome(result.path);
        if (displayedPath.current !== result.path) {
          setQuery("");
          setSelected(undefined);
        }
        displayedPath.current = result.path;
        requestedPath.current = result.path;
        setData(result);
        directoryChanged.current(result.path);
      })
      .catch((error) => {
        if (!active) return;
        requestedPath.current = displayedPath.current ?? path;
        setError({ path, message: explain(error) });
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => { active = false; };
  }, [space, reload, revision]);

  const open = (next: string) => {
    if (loading) return;
    requestedPath.current = next;
    setLocation(undefined);
    setLoading(true);
    setReload((n) => n + 1);
  };
  const openEntry = (entry: Entry) => {
    if (loading) return;
    if (entry.kind === "file") setEditing(entry);
    else if (isDirectory(entry)) open(entry.path);
  };
  const crumbs = [{ name: "/", path: "/" }];
  for (const name of data?.path.split("/").filter(Boolean) ?? []) {
    const parent = crumbs.at(-1)!.path;
    crumbs.push({ name, path: parent === "/" ? "/" + name : parent + "/" + name });
  }
  const parent = crumbs.at(-2)?.path;
  useEffect(() => {
    const trail = breadcrumbs.current;
    if (trail) trail.scrollLeft = trail.scrollWidth;
  }, [data?.path, location]);

  const prepare = (next: Action) => {
    setAction(next);
    setName(next.kind === "chmod" && next.entry ? permissions(next.entry) : next.entry?.name ?? "");
    setActionError("");
  };
  const validName = action?.kind === "chmod" ? /^[0-7]{3,4}$/.test(name) :
    !!name.trim() &&
    name !== "." &&
    name !== ".." &&
    !name.includes("/") &&
    !name.includes("\0");
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!action || saving || !writable || (!data && !action.entry)) return;
    setSaving(true);
    setActionError("");
    const parent = action.entry
      ? action.entry.path.split("/").slice(0, -1).join("/")
      : data!.path;
    const destination = `${parent.replace(/\/$/, "")}/${name}`;
    try {
      await call("file_action", {
        space,
        action: action.kind,
        path: action.kind === "mkdir" ? destination : action.entry!.path,
        destination: action.kind === "rename" ? destination : action.kind === "chmod" ? name : null,
      });
      setAction(undefined);
      if (action.kind !== "chmod" && action.entry?.path === selected) setSelected(undefined);
      setReload((n) => n + 1);
    } catch (e) {
      setActionError(explain(e));
    } finally {
      setSaving(false);
    }
  };
  const entries = useMemo(() => arrangeFiles(data?.entries ?? [], query, hidden, sort), [data, query, hidden, sort]);
  const key = `${data?.path}\0${query}\0${hidden}\0${sort}`;
  const [paging, setPaging] = useState({ key, page: 0 });
  const visible = filePage(entries, paging.key === key ? paging.page : 0);
  const selection = visible.entries.find((entry) => entry.path === selected);
  useEffect(() => {
    if (list.current) list.current.scrollTop = 0;
  }, [key, visible.page]);

  return (
    <section className="files-page" aria-label="远端文件">
      <div className="file-toolbar">
        <div className="file-navigation">
          <button className="icon-button" aria-label="上一级" title="上一级"
            disabled={loading || !parent} onClick={() => open(parent!)}>
            <Icon name="up" size={17} />
          </button>
          {location !== undefined ? (
            <form className="file-location" onSubmit={(event) => {
              event.preventDefault();
              if (location.trim()) open(location);
            }}>
              <input autoFocus aria-label="远端目录路径" value={location}
                onFocus={(event) => event.target.select()}
                onChange={(event) => setLocation(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Escape") setLocation(undefined);
                }} />
              <button type="submit" disabled={loading || !location.trim()}>前往</button>
              <button type="button" className="icon-button" aria-label="取消输入路径"
                onClick={() => setLocation(undefined)}><Icon name="close" size={14} /></button>
            </form>
          ) : (
            <div className="file-location">
              <nav ref={breadcrumbs} className="file-breadcrumbs" aria-label="当前目录路径" title={data?.path}>
                {data ? crumbs.map((crumb, i) => (
                  <span key={crumb.path}>
                    {i > 0 && <Icon name="chevron" size={12} />}
                    {i === crumbs.length - 1 ? (
                      <span aria-current="location">{crumb.name}</span>
                    ) : <button disabled={loading} onClick={() => open(crumb.path)}>{crumb.name}</button>}
                  </span>
                )) : <span>{loading ? "正在读取目录…" : "远端目录"}</span>}
              </nav>
              <button className="icon-button" aria-label="输入路径" title="输入路径"
                disabled={loading} onClick={() => setLocation(data?.path ?? requestedPath.current)}>
                <Icon name="edit" size={14} />
              </button>
            </div>
          )}
          <button className="icon-button" aria-label="刷新文件" title="刷新文件"
            disabled={loading} onClick={() => open(data?.path ?? requestedPath.current)}>
            <Icon name="refresh" size={16} />
          </button>
        </div>
        <div className="file-toolbar-actions">
          <button disabled={!writable || !data || loading} onClick={() => prepare({ kind: "mkdir" })}>
            <Icon name="plus" size={15} />新建文件夹
          </button>
          <button className="primary" disabled={!writable || !data || loading}
            onClick={() => void transfer("upload", space, data!.path)}>
            <Icon name="upload" size={15} />上传文件
          </button>
          <label className="file-more" data-disabled={!data || loading}>
            <span>更多</span><Icon name="chevron" size={13} />
            <select aria-label="当前目录操作" value="" disabled={!data || loading} onChange={event => {
              if (event.target.value === "compare") setComparing(data!.path);
              if (event.target.value === "folder") void transfer("upload", space, data!.path, true);
            }}>
              <option value="" disabled>更多操作</option>
              <option value="folder" disabled={!writable}>上传文件夹…</option>
              <option value="compare">比较本地与远端目录…</option>
            </select>
          </label>
        </div>
      </div>
      {error && (
        <div className="file-error error-banner" role="alert">
          <span title={error.path}>无法打开 {error.path} · {error.message}</span>
          <button onClick={() => open(error.path)}>重试</button>
        </div>
      )}
      <div className="file-browser">
        <nav className="file-places" aria-label="服务器位置">
          <span className="file-places-heading">位置</span>
          <button aria-current={data?.path === "/" ? "location" : undefined}
            disabled={loading} onClick={() => open("/")}>
            <Icon name="server" size={15} />根目录 <code>/</code>
          </button>
          <button aria-current={home && home !== "/" && data?.path === home ? "location" : undefined}
            disabled={loading} onClick={() => open(".")}>
            <Icon name="home" size={15} />主目录
          </button>
        </nav>
        <div className="file-directory" aria-busy={loading}>
          <div className="file-options">
            <label className="file-filter">
              <Icon name="search" size={15} />
              <input placeholder="筛选文件…" aria-label="筛选文件" value={query} onChange={event => {
                setQuery(event.target.value);
                setSelected(undefined);
              }} />
            </label>
            <label className="check-label"><input type="checkbox" checked={hidden} onChange={event => {
              setHidden(event.target.checked);
              setSelected(undefined);
            }} />隐藏文件</label>
            <label className="file-sort">
              <select aria-label="文件排序" value={sort} onChange={event => {
                setSort(event.target.value as FileSort);
                setSelected(undefined);
              }}>
                <option value="name-asc">名称 ↑</option><option value="name-desc">名称 ↓</option>
                <option value="modified-desc">最近修改</option><option value="modified-asc">最早修改</option>
                <option value="size-desc">最大文件</option><option value="size-asc">最小文件</option>
              </select>
            </label>
          </div>
          <div className="file-table-wrap" ref={list} tabIndex={0} aria-label="当前目录文件列表">
            {visible.entries.length ? (
              <table className="file-table" aria-label={data?.path}>
                <thead><tr>
                  <th scope="col">名称</th>
                  <th scope="col">大小</th>
                  <th scope="col">修改时间</th>
                  <th scope="col">权限</th>
                </tr></thead>
                <tbody>
                  {visible.entries.map((entry) => (
                    <tr key={entry.path} data-selected={selection?.path === entry.path}
                      onClick={() => { if (!loading) setSelected(entry.path); }}
                      onDoubleClick={() => openEntry(entry)}>
                      <td>
                        <button className="file-name" title={entry.path} data-preview-path={entry.path}
                          aria-pressed={selection?.path === entry.path} disabled={loading}
                          onKeyDown={event => {
                            if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                              event.preventDefault();
                              if (isDirectory(entry)) list.current?.focus({ preventScroll: true });
                              setSelected(entry.path);
                              openEntry(entry);
                            }
                          }}>
                          <Icon name={entry.kind === "directory" ? "folder" : entry.kind === "symlink" ? "link" : "file"} size={17} />
                          <span>{entry.name}</span>
                        </button>
                      </td>
                      <td>{entry.kind === "directory" ? "—" : bytes(entry.size)}</td>
                      <td>{modified(entry)}</td>
                      <td><code>{permissions(entry)}</code></td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <div className="file-empty">
                <Icon name="folder" size={30} />
                <p>{loading ? "正在读取目录…" : !data ? "目录暂不可用" : query ? "没有匹配的文件" : "这个目录没有可显示的文件"}</p>
              </div>
            )}
          </div>
          <footer className="file-list-status">
            <span role="status">
              {loading ? "读取中…" : visible.pages > 1
                ? `${visible.start + 1}–${visible.start + visible.entries.length} / ${entries.length} 项`
                : `${entries.length} 个项目`}
              {data?.truncated && " · 已达显示上限"}
              {!writable && " · 只读"}
            </span>
            {visible.pages > 1 && (
              <nav className="file-pagination" aria-label="文件分页">
                <button className="icon-button" aria-label="上一页文件" disabled={loading || visible.page === 0}
                  onClick={() => { setPaging({ key, page: visible.page - 1 }); setSelected(undefined); }}>
                  <Icon name="back" size={14} />
                </button>
                <span>{visible.page + 1}/{visible.pages}</span>
                <button className="icon-button" aria-label="下一页文件" disabled={loading || visible.page === visible.pages - 1}
                  onClick={() => { setPaging({ key, page: visible.page + 1 }); setSelected(undefined); }}>
                  <Icon name="chevron" size={14} />
                </button>
              </nav>
            )}
            {selection && <div className="file-selection-actions">
              {selection.kind !== "other" && (
                <button className="icon-button" title="打开" aria-label={`打开 ${selection.name}`} disabled={loading}
                  onClick={() => openEntry(selection)}>
                  <Icon name={selection.kind === "file" ? "edit" : "folder"} size={15} />
                </button>
              )}
              {(selection.kind === "directory" || selection.kind === "file") && (
                <button className="icon-button" title="下载" aria-label={`下载 ${selection.name}`} disabled={loading}
                  onClick={() => void transfer("download", space, selection.path, selection.kind === "directory")}>
                  <Icon name="download" size={15} />
                </button>
              )}
              {writable && (
                <label className="file-more" data-disabled={loading}>
                  <span>管理</span><Icon name="chevron" size={12} />
                  <select aria-label={`管理 ${selection.name}`} value="" disabled={loading} onChange={event => {
                    const kind = event.target.value as Action["kind"];
                    prepare({ kind, entry: selection });
                  }}>
                    <option value="" disabled>管理</option>
                    <option value="rename">重命名…</option>
                    {selection.kind === "file" && <option value="chmod">修改权限…</option>}
                    <option value="delete">删除…</option>
                  </select>
                </label>
              )}
            </div>}
          </footer>
        </div>
      </div>
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
          writable={writable}
          onApplied={() => setReload((n) => n + 1)}
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
                  : action.kind === "chmod" ? "修改文件权限" : "删除文件或空文件夹？"}
            </h2>
            {action.kind === "delete" ? (
              <p className="file-delete-path">
                {action.entry!.path}
                <br />
                此操作无法撤销；非空文件夹不会被递归删除。
              </p>
            ) : (
              <label className="file-edit-label">
                {action.kind === "chmod" ? "权限（八进制）" : "名称"}
                <input
                  autoFocus
                  aria-label={action.kind === "chmod" ? "文件权限" : "文件名称"}
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  disabled={saving}
                />
                <small>{action.kind === "chmod" ? "例如 640。仅修改当前文件。" : "已有同名项目不会被覆盖。"}</small>
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
