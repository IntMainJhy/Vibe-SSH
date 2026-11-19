import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { call, explain, type Space } from "./api";
import { Icon } from "./Icon";

export const bytes = (size: number | null | undefined) => {
  if (size == null) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit++;
  }
  return `${size.toFixed(unit ? 1 : 0)} ${units[unit]}`;
};
type Transfer = {
  id: string;
  name: string;
  direction: "upload" | "download";
  done: number;
  total: number | null;
  unit?: "entries";
  resumable?: boolean;
  status: "running" | "completed" | "failed" | "cancelled";
  error?: string;
};
type PendingDownload = {
  id: string;
  name: string;
  host: string;
  done: number;
  total: number;
};
type PendingUpload = Omit<PendingDownload, "done"> & { staging: string };
type PendingFolder = PendingUpload & { direction: Transfer["direction"]; folder: true };
type Pending = (PendingDownload & { direction: "download" }) | (PendingUpload & { direction: "upload" }) | PendingFolder;

export function useTransfers() {
  const [items, setItems] = useState<Transfer[]>([]);
  const [pending, setPending] = useState<Pending[]>([]);
  const [queueError, setQueueError] = useState("");
  const [revision, setRevision] = useState(0);
  const queueRequest = useRef(0);
  const loadPending = useCallback(async () => {
    const request = ++queueRequest.current;
    const [downloads, uploads, folders] = await Promise.allSettled([
      call<PendingDownload[]>("pending_downloads"),
      call<PendingUpload[]>("pending_uploads"),
      call<PendingFolder[]>("pending_folders"),
    ]);
    if (request !== queueRequest.current) return;
    setPending((previous) => [
      ...(downloads.status === "fulfilled"
        ? downloads.value.map((item) => ({ ...item, direction: "download" as const }))
        : previous.filter((item) => !("folder" in item) && item.direction === "download")),
      ...(uploads.status === "fulfilled"
        ? uploads.value.map((item) => ({ ...item, direction: "upload" as const }))
        : previous.filter((item) => !("folder" in item) && item.direction === "upload")),
      ...(folders.status === "fulfilled"
        ? folders.value
        : previous.filter((item) => "folder" in item)),
    ]);
    setQueueError([downloads, uploads, folders].flatMap((result) =>
      result.status === "rejected" ? [explain(result.reason)] : [],
    ).join(" · "));
  }, []);
  useEffect(() => {
    void loadPending();
  }, [loadPending]);
  useEffect(() => {
    const off = listen<Transfer>("file-transfer", ({ payload }) =>
      setItems((old) =>
        old.map((item) =>
          item.id === payload.id && item.status === "running"
            ? { ...item, ...payload }
            : item,
        ),
      ),
    );
    return () => {
      void off.then((stop) => stop()).catch(() => {});
    };
  }, []);
  const execute = async (
    item: Transfer,
    command: string,
    args: Record<string, unknown>,
  ) => {
    const { id } = item;
    setItems((old) => [
      ...old.filter((item) => item.id !== id && item.status === "running"),
      item,
    ]);
    try {
      const selected = await call<boolean>(command, { id, ...args });
      setItems((old) =>
        old.map((item) =>
          item.id === id
            ? { ...item, status: selected ? "completed" : "cancelled" }
            : item,
        ),
      );
    } catch (error) {
      setItems((old) =>
        old.map((item) =>
          item.id === id
            ? { ...item, status: "failed", error: explain(error) }
            : item,
        ),
      );
    } finally {
      void loadPending();
      setRevision((n) => n + 1);
    }
  };

  const transfer = (direction: "upload" | "download", space: string, path: string, folder = false) =>
    execute(
      {
        id: crypto.randomUUID(),
        name: direction === "upload"
          ? folder ? "选择上传文件夹…" : "选择上传文件…"
          : path.split("/").at(-1)!,
        direction, done: 0, total: null, status: "running",
        resumable: direction === "download" && !folder,
      },
      folder ? "transfer_folder" : direction === "upload" ? "upload_files" : "download_file",
      { space, ...(folder ? { remote: path, upload: direction === "upload" }
        : direction === "upload" ? { directory: path } : { remote: path }) },
    );

  const dismiss = (id: string) => setItems((old) => old.filter((item) => item.id !== id));
  return {
    items,
    pending,
    queueError,
    transfer,
    revision,
    active: items.filter((item) => item.status === "running").length,
    dismiss,
    resume: (item: Pending, space: string) =>
      execute(
        { ...item, done: "done" in item ? item.done : 0, status: "running", resumable: true },
        "folder" in item ? "resume_folder" : item.direction === "download" ? "resume_download" : "resume_upload",
        { space },
      ),
    discard: async (item: Pending) => {
      const { id, direction } = item;
      try {
        await call("folder" in item ? "discard_folder" : direction === "download" ? "discard_download" : "discard_upload", { id });
        dismiss(id);
        await loadPending();
      } catch (error) {
        setQueueError(explain(error));
      }
    },
  };
}
export type TransferStart = ReturnType<typeof useTransfers>["transfer"];
export function TransferTray({
  state,
  spaces,
  selected,
}: {
  state: ReturnType<typeof useTransfers>;
  spaces: Space[];
  selected: string | null;
}) {
  const pending = state.pending.filter((download) =>
    !state.items.some((item) => item.id === download.id && item.status === "running"),
  );
  const items = state.items.filter((item) => !pending.some((download) => download.id === item.id));
  if (!items.length && !pending.length && !state.queueError) return null;
  return (
    <section className="transfer-tray" aria-label="文件传输" aria-live="polite">
      {state.queueError && <p className="transfer-item transfer-error">{state.queueError}</p>}
      {items.map((item) => (
        <div className="transfer-item" key={item.id}>
          <div className="transfer-title">
            <Icon
              name={item.direction === "upload" ? "upload" : "download"}
              size={16}
            />
            <strong>{item.name}</strong>
            <button
              className="icon-button"
              aria-label={
                item.status === "running"
                  ? item.resumable ? item.direction === "upload" ? "暂停上传" : "暂停下载" : "取消传输"
                  : "关闭传输记录"
              }
              title={item.status === "running" && item.resumable ? (item.direction === "upload" ? "暂停上传" : "暂停下载") : undefined}
              onClick={() =>
                item.status === "running"
                  ? void call("cancel_transfer", { id: item.id })
                  : state.dismiss(item.id)
              }
            >
              <Icon name="close" size={14} />
            </button>
          </div>
          <p>
            {item.status === "running"
              ? item.unit === "entries" ? `已传输 ${item.done} 项` : `${bytes(item.done)} / ${bytes(item.total)}`
              : item.status === "completed"
                ? "传输完成"
                : item.status === "cancelled"
                  ? "已取消选择"
                  : "传输未完成"}
          </p>
          {item.status === "running" && (
            <progress
              aria-label="传输进度"
              max={item.total || 1}
              value={item.total ? item.done : undefined}
            />
          )}
          {item.error && <p className="transfer-error">{item.error}</p>}
        </div>
      ))}
      {pending.map((item) => {
        const available = spaces.filter((space) => space.connected && space.host === item.host && (item.direction === "download" || space.control.owner === "human"));
        const space = available.find((space) => space.space === selected) ?? available[0];
        const error = state.items.find((active) => active.id === item.id)?.error;
        return (
          <div className="transfer-item" key={item.id}>
            <div className="transfer-title">
              <Icon name={item.direction} size={16} />
              <strong title={item.name}>{item.name}</strong>
            </div>
            <p>{"done" in item ? `${bytes(item.done)} / ` : ""}{bytes(item.total)} · 等待续传</p>
            {error && <p className="transfer-error">{error}</p>}
            {!space && <p>{item.direction === "upload" ? "请在原服务器的个人会话或已接管空间继续。" : "连接原服务器后可继续。"}</p>}
            {"staging" in item && <p title={item.staging}>放弃续传会保留临时{ "folder" in item ? "目录" : "文件"}。</p>}
            <div className="transfer-actions">
              <button className="primary" disabled={!space} onClick={() => space && void state.resume(item, space.space)}>
                {item.direction === "upload" ? "继续上传" : "继续下载"}
              </button>
              <button onClick={() => void state.discard(item)}>放弃续传</button>
            </div>
          </div>
        );
      })}
    </section>
  );
}
