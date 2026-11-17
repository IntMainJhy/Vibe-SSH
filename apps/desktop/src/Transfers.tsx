import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { call, explain } from "./api";
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
  status: "running" | "completed" | "failed" | "cancelled";
  error?: string;
};
export function useTransfers() {
  const [items, setItems] = useState<Transfer[]>([]);
  const [revision, setRevision] = useState(0);
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
  const transfer = async (
    direction: "upload" | "download",
    space: string,
    path: string,
  ) => {
    const id = crypto.randomUUID();
    setItems((old) => [
      ...old.filter((item) => item.status === "running").slice(-31),
      {
        id,
        name:
          direction === "upload" ? "选择上传文件…" : path.split("/").at(-1)!,
        direction,
        done: 0,
        total: null,
        status: "running",
      },
    ]);
    try {
      const selected = await call<boolean>(
        direction === "upload" ? "upload_files" : "download_file",
        {
          id,
          space,
          ...(direction === "upload" ? { directory: path } : { remote: path }),
        },
      );
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
      setRevision((n) => n + 1);
    }
  };
  return {
    items,
    transfer,
    revision,
    active: items.filter((item) => item.status === "running").length,
    dismiss: (id: string) =>
      setItems((old) => old.filter((item) => item.id !== id)),
  };
}
export type TransferStart = ReturnType<typeof useTransfers>["transfer"];
export function TransferTray({
  state,
}: {
  state: ReturnType<typeof useTransfers>;
}) {
  if (!state.items.length) return null;
  return (
    <section className="transfer-tray" aria-label="文件传输" aria-live="polite">
      {state.items.map((item) => (
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
                item.status === "running" ? "取消传输" : "关闭传输记录"
              }
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
              ? `${bytes(item.done)} / ${bytes(item.total)}`
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
    </section>
  );
}
