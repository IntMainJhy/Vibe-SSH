export type Entry = {
  name: string;
  path: string;
  kind: "file" | "directory" | "symlink" | "other";
  size: number | null;
  modified: number | null;
  permissions: number | null;
};
export type FileSort = "name-asc" | "name-desc" | "modified-asc" | "modified-desc" | "size-asc" | "size-desc";
export const FILE_PAGE_SIZE = 100;
const names = new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" });
const dates = new Intl.DateTimeFormat("zh-CN", {
  month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit",
});
export function fileModified(seconds: number | null) {
  const millis = seconds == null ? NaN : seconds * 1000;
  return Number.isFinite(millis) && Math.abs(millis) <= 8.64e15 ? dates.format(millis) : "—";
}

/** Sort metadata already read from the server; never read file contents. */
export function arrangeFiles(entries: Entry[], query: string, hidden: boolean, sort: FileSort) {
  const needle = query.toLocaleLowerCase();
  const [field, order] = sort.split("-");
  const direction = order === "desc" ? -1 : 1;
  return entries.filter(entry =>
    (hidden || !entry.name.startsWith(".")) && entry.name.toLocaleLowerCase().includes(needle),
  ).sort((a, b) => {
    const directory = Number(b.kind === "directory") - Number(a.kind === "directory");
    if (directory) return directory;
    if (field === "size" || field === "modified") {
      const left = a[field], right = b[field];
      if (left === null && right !== null) return 1;
      if (right === null && left !== null) return -1;
      if (left !== null && right !== null && left !== right) return (left - right) * direction;
    }
    const name = names.compare(a.name, b.name) || a.path.localeCompare(b.path);
    return field === "name" ? name * direction : name;
  });
}

export function filePage(entries: Entry[], requested: number) {
  const pages = Math.max(1, Math.ceil(entries.length / FILE_PAGE_SIZE));
  const page = Math.max(0, Math.min(requested, pages - 1));
  const start = page * FILE_PAGE_SIZE;
  return { page, pages, start, entries: entries.slice(start, start + FILE_PAGE_SIZE) };
}
