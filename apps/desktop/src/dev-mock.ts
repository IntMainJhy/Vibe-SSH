// Sample data for developing the interface in a plain browser (`npm run dev`), where the
// app shell is absent. Loaded only in development; it is not part of the production bundle.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { Host, Snapshot, Space } from "./api";

const key = "SHA256:nThbg6kXUpJWGl7E1IGOCspRomTxdCARLviKw6E5SY8";

let counter = 10;
const hosts: Host[] = [
  {
    id: "blog",
    name: "博客服务器",
    host: "203.0.113.10",
    port: 22,
    username: "deploy",
    identity_file: "/Users/me/.ssh/id_ed25519",
    host_key_sha256: key,
    agent_access: true,
  },
  {
    id: "db",
    name: "数据库",
    host: "203.0.113.20",
    port: 2222,
    username: "root",
    identity_file: null,
    host_key_sha256: key,
    agent_access: false,
  },
];
const job = (
  id: string,
  command: string | null,
  status: Space["jobs"][0]["status"],
  pty = false,
) => ({
  job: id,
  command,
  pty,
  status,
  started_unix_ms: Date.now(),
  exit_code: status === "succeeded" ? 0 : status === "failed" ? 1 : null,
});
const spaces: Space[] = [
  {
    space: "s1",
    host: "blog",
    name: "部署博客 v2",
    host_name: "博客服务器",
    actor: "Agent",
    origin: "agent",
    connected: true,
    control: { owner: "agent", note: null },
    jobs: [
      job("j1", "npm ci && npm run build", "succeeded"),
      job("j2", "nginx -t", "failed"),
      job("j3", "docker compose up --build", "running"),
    ],
  },
  {
    space: "s3",
    host: "db",
    name: "终端",
    host_name: "数据库",
    actor: "me",
    origin: "human",
    connected: true,
    control: { owner: "human", note: null },
    jobs: [job("j5", null, "running", true)],
  },
];
const output: Record<string, [string, string]> = {
  j1: [
    "added 312 packages in 9s\n\n> blog@2.0.0 build\n> vite build\n\n✓ 148 modules transformed.\ndist/index.html   0.62 kB\n✓ built in 2.31s\n",
    "",
  ],
  j2: [
    "",
    'nginx: [emerg] unknown directive "servr_name" in /etc/nginx/sites-enabled/blog:4\nnginx: configuration file /etc/nginx/nginx.conf test failed\n',
  ],
  j3: [
    "[+] Building 12.4s (9/14)\n => [web 3/6] COPY package*.json ./\n => [web 4/6] RUN npm ci --omit=dev\n",
    "",
  ],
  j4: ["[sudo] password for deploy: ", ""],
  j5: [
    "Last login: Mon Nov 16 21:04:11 2026 from 198.51.100.7\r\nroot@db:~# ",
    "",
  ],
};
const bytes = (text: string) => Array.from(new TextEncoder().encode(text));
const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms));
const find = (id: string) => spaces.find((s) => s.space === id)!;

const scenario = new URLSearchParams(location.search).get("scenario");
if (scenario === "empty") {
  hosts.length = 0;
  spaces.length = 0;
}
if (scenario === "handoff")
  spaces[0].control = {
    owner: "human",
    note: "sudo 需要你的密码，请在终端中完成验证。",
  };
if (scenario === "disconnected") spaces[spaces.length - 1].connected = false;
document.documentElement.dataset.preview = "true";
mockWindows("main");
const fileEntries = [
  {
    name: "app",
    path: "/home/deploy/app",
    kind: "directory",
    size: 4096,
    modified: 1794901800,
    permissions: 0o40755,
  },
  {
    name: "releases",
    path: "/home/deploy/releases",
    kind: "directory",
    size: 4096,
    modified: 1794901800,
    permissions: 0o40755,
  },
  {
    name: ".profile",
    path: "/home/deploy/.profile",
    kind: "file",
    size: 807,
    modified: 1794801800,
    permissions: 0o100644,
  },
  {
    name: "compose.yaml",
    path: "/home/deploy/compose.yaml",
    kind: "file",
    size: 1248,
    modified: 1794901800,
    permissions: 0o100644,
  },
  {
    name: "server.log",
    path: "/home/deploy/server.log",
    kind: "file",
    size: 146_928,
    modified: 1794901800,
    permissions: 0o100600,
  },
  {
    name: "current",
    path: "/home/deploy/current",
    kind: "symlink",
    size: 16,
    modified: 1794901800,
    permissions: 0o120777,
  },
];
const fileTexts: Record<string, string> = {
  "/home/deploy/app/config/nginx.conf": "server {\n    listen 80;\n    server_name example.test;\n}\n",
  "/home/deploy/app/package.json": '{"name":"blog","scripts":{"build":"vite build"}}\n',
  "/home/deploy/app/README.md": "# Blog\n\nBuild with npm run build.\n",
  "/home/deploy/app/src/index.ts": 'console.log("ready");\n',
  "/home/deploy/compose.yaml":
    'services:\n  web:\n    image: nginx:alpine\n    ports:\n      - "8080:80"\n    restart: unless-stopped\n',
  "/home/deploy/server.log": "2026-11-17T12:00:00Z server ready\n",
  "/home/deploy/.profile": 'export PATH="$HOME/.local/bin:$PATH"\n',
};
for (const [path, kind] of [
  ["/home", "directory"],
  ["/home/deploy", "directory"],
  ["/home/deploy/app/config", "directory"],
  ["/home/deploy/app/src", "directory"],
  ["/home/deploy/releases/2026-11-17", "directory"],
  ["/home/deploy/app/config/nginx.conf", "file"],
  ["/home/deploy/app/package.json", "file"],
  ["/home/deploy/app/README.md", "file"],
  ["/home/deploy/app/src/index.ts", "file"],
  ["/home/deploy/app/.env", "file"],
] as const) {
  fileEntries.push({
    path, kind, name: path.split("/").at(-1)!,
    size: kind === "directory" ? 4096 : new TextEncoder().encode(fileTexts[path] ?? "").length,
    modified: 1794901800,
    permissions: kind === "directory" ? 0o40755 : 0o100644,
  });
}
if (scenario === "large-directory") {
  fileEntries.push({ name: "large", path: "/home/deploy/large", kind: "directory", size: 4096, modified: 1794901800, permissions: 0o40755 });
  for (let i = 5000; i > 0; i--) {
    fileEntries.push({ name: `file-${i}.txt`, path: `/home/deploy/large/file-${i}.txt`, kind: "file", size: i * 17, modified: 1794901800 + i, permissions: 0o100644 });
  }
}
let resourceCalls = 0;
const documents = new Map<
  string,
  { path: string; text: string; revision: string; save_supported: boolean }
>();
let conflicted = false;
const historyRecords = spaces.flatMap((s) =>
  s.jobs.map((j, i) => ({
    record: `h${j.job.padStart(32, "0")}`,
    host_name: s.host_name,
    space_name: s.name,
    actor: s.actor,
    origin: s.origin,
    command: j.command,
    command_truncated: false,
    pty: j.pty,
    started_unix_ms: Date.now() - (i + 1) * 60_000,
    elapsed_ms: 12400,
    status: j.status === "running" ? "unknown" : j.status,
    reason: j.status === "running" ? "client_restarted" : "exit_status",
    exit_code: j.exit_code,
  })),
);
const diffManifest = {
  entries: [
    { path: "src/index.ts", kind: "file", size: 2840, excluded: false },
    { path: "package.json", kind: "file", size: 620, excluded: false },
    { path: "README.md", kind: "file", size: 1240, excluded: false },
    { path: ".env.production", kind: "file", size: 76, excluded: true },
    { path: "node_modules", kind: "directory", size: null, excluded: true },
    { path: "current", kind: "symlink", size: null, excluded: false },
  ],
  excludes: [
    ".git",
    "node_modules",
    "target",
    "dist",
    ".ssh",
    ".env*",
    "*.pem",
    "*.key",
    "*.p12",
  ],
  truncated: false,
};
mockIPC(
  async (command, payload) => {
    const args = (payload ?? {}) as Record<string, any>;
    switch (command) {
      case "history_list":
        if (scenario === "history-error")
          throw { message: "任务记录不可用：数据库无法读取，请检查数据目录。" };
        return {
          records:
            scenario === "empty"
              ? []
              : historyRecords.slice(args.offset, args.offset + args.limit),
          more: false,
        };
      case "history_get": {
        const record = historyRecords.find((r) => r.record === args.record)!;
        const job = args.record.slice(-2);
        const streams = output[job] ?? ["", ""];
        return {
          ...record,
          stdout: {
            text: streams[0],
            total_bytes: new TextEncoder().encode(streams[0]).length,
            truncated: false,
            invalid_utf8: false,
          },
          stderr: {
            text: streams[1],
            total_bytes: new TextEncoder().encode(streams[1]).length,
            truncated: false,
            invalid_utf8: false,
          },
        };
      }
      case "choose_diff":
        return {
          id: "preview-source",
          name: "/Projects/博客站点",
          manifest: structuredClone(diffManifest),
        };
      case "forget_diff":
        return;
      case "rescan_diff":
        if (args.excludes.some((s: string) => s.includes("/")))
          throw { message: "忽略规则不能包含 /。" };
        return {
          ...structuredClone(diffManifest),
          excludes: [...diffManifest.excludes, ...args.excludes],
        };
      case "compare_directory":
        await sleep(150);
        if (scenario === "diff-error")
          throw { message: "比较期间服务器连接断开，请核实连接后重试。" };
        return {
          remote: args.remote,
          truncated: false,
          entries: diffManifest.entries.map((e) => ({
            path: e.path,
            local_size: e.size,
            remote_size: e.path === "README.md" ? null : e.size,
            status: e.excluded
              ? "excluded"
              : e.kind === "symlink"
                ? "unsupported"
                : e.path === "README.md"
                  ? "local_only"
                  : !args.selected.includes(e.path)
                    ? "unverified"
                    : e.path === "src/index.ts"
                      ? "changed"
                      : "same",
          })),
        };
      case "snapshot":
        if (scenario === "offline")
          throw { code: "SERVICE_UNAVAILABLE", message: "preview unavailable" };
        return structuredClone({
          status: {
            version: "0.2.0-preview.5",
            protocol_version: 2,
            instance: "preview",
          },
          open_documents: documents.size,
          hosts,
          spaces,
          skill: { skill: true, cli: true },
        } satisfies Snapshot);
      case "files": {
        await sleep(scenario === "files-race" && args.path === "/home/deploy/app" ? 650 : 100);
        let path =
          args.path === "."
            ? "/home/deploy"
            : args.path.replace(/\/$/, "") || "/";
        if (scenario === "files-denied" && path === "/home/deploy/app/config")
          throw { message: "没有读取这个目录的权限。" };
        if (path === "/home/deploy/current") path = "/home/deploy/app";
        if (
          path !== "/home/deploy" &&
          !fileEntries.some(
            (e) => e.path === path && ["directory", "symlink"].includes(e.kind),
          ) &&
          path !== "/" &&
          path !== "/home"
        )
          throw { message: "目录或文件不存在，请刷新后重试。" };
        return {
          path,
          entries: fileEntries.filter(
            (e) => e.path.slice(0, e.path.lastIndexOf("/")) === path,
          ),
          truncated: false,
        };
      }
      case "text_file": {
        await sleep(120);
        if (!(args.path in fileTexts))
          throw { message: "此文件不是可预览的 UTF-8 文本。" };
        const doc = {
          path: args.path,
          text: fileTexts[args.path],
          revision: `doc-${++counter}`,
          save_supported: true,
        };
        documents.set(doc.revision, doc);
        void emit("changed");
        return structuredClone(doc);
      }
      case "forget_text":
        documents.delete(args.revision);
        void emit("changed");
        return;
      case "save_text": {
        await sleep(150);
        if (
          spaces.find((s) => s.space === args.space)?.control.owner !== "human"
        )
          throw { code: "CONTROL_REVOKED" };
        const doc = documents.get(args.revision);
        if (!doc) throw { message: "文件版本已过期，请重新打开" };
        if (scenario === "editor-conflict" && !conflicted) {
          fileTexts[doc.path] += "# changed by another process\n";
          conflicted = true;
        }
        if (fileTexts[doc.path] !== doc.text)
          throw {
            code: "FILE_CHANGED",
            message:
              "远端文件已被修改，未覆盖任何内容。草稿已保留，请复制草稿或重新读取远端版本。",
          };
        const backup = `/home/deploy/.vibe-backup-${++counter}`;
        fileTexts[backup] = doc.text;
        fileTexts[doc.path] = args.text;
        const entry = fileEntries.find((e) => e.path === doc.path);
        if (entry) {
          entry.size = new TextEncoder().encode(args.text).length;
          entry.modified = Date.now() / 1000;
        }
        documents.delete(doc.revision);
        const next = { ...doc, text: args.text, revision: `doc-${++counter}` };
        documents.set(next.revision, next);
        void emit("changed");
        return { document: structuredClone(next), backup };
      }
      case "resources":
        await sleep(150);
        if (scenario === "resources-recover" && ++resourceCalls === 1)
          throw { message: "采样暂时不可用，连接正在恢复。" };
        return {
          cpu_percent: 12.4,
          cores: 4,
          memory_total: 8 * 1024 ** 3,
          memory_available: 5.2 * 1024 ** 3,
          swap_total: 2 * 1024 ** 3,
          swap_free: 2 * 1024 ** 3,
          disk_total: 80 * 1024 ** 3,
          disk_used: 26.8 * 1024 ** 3,
          disk_available: 49.2 * 1024 ** 3,
          uptime_s: 998284,
          load: [0.42, 0.38, 0.31],
        };
      case "file_action": {
        if (
          spaces.find((s) => s.space === args.space)?.control.owner !== "human"
        )
          throw { code: "CONTROL_REVOKED" };
        const exists = fileEntries.find((e) => e.path === args.path);
        if (args.action === "mkdir") {
          if (exists) throw { message: "已有同名文件夹" };
          fileEntries.push({
            name: args.path.split("/").at(-1),
            path: args.path,
            kind: "directory",
            size: 4096,
            modified: Date.now() / 1000,
            permissions: 0o40755,
          });
        } else if (args.action === "rename" && exists) {
          exists.path = args.destination;
          exists.name = args.destination.split("/").at(-1);
        } else if (args.action === "delete" && exists) {
          fileEntries.splice(fileEntries.indexOf(exists), 1);
        }
        return;
      }
      case "upload_files":
      case "download_file":
        return false;
      case "cancel_transfer":
        return;
      case "host_key":
        await sleep(400);
        return ["ssh-ed25519", key];
      case "save_host": {
        const host: Host = {
          ...args.host,
          id: args.host.id || `h${++counter}`,
        };
        const index = hosts.findIndex((h) => h.id === host.id);
        if (index === -1) hosts.push(host);
        else hosts[index] = host;
        await emit("changed");
        return host;
      }
      case "delete_host":
        hosts.splice(
          hosts.findIndex((h) => h.id === args.id),
          1,
        );
        break;
      case "open_terminal": {
        await sleep(250);
        if (scenario === "connect-error") throw { code: "CONNECTION_FAILED" };
        const host = hosts.find((h) => h.id === args.host)!;
        const id = `s${++counter}`,
          jid = `j${++counter}`;
        spaces.push({
          space: id,
          host: host.id,
          name: "终端",
          host_name: host.name,
          actor: "me",
          origin: "human",
          connected: true,
          control: { owner: "human", note: null },
          jobs: [job(jid, null, "running", true)],
        });
        output[jid] = [`${host.username}@${host.name}:~$ `, ""];
        await emit("changed");
        return { space: id, job: jid };
      }
      case "shell": {
        const id = `j${++counter}`;
        find(args.space).jobs.push(job(id, null, "running", true));
        output[id] = ["user@host:~$ ", ""];
        await emit("changed");
        return id;
      }
      case "job_input": {
        const s = find(args.space);
        if (s.control.owner !== "human") throw { code: "CONTROL_REVOKED" };
        output[args.job][0] += args.data;
        return;
      }
      case "job_resize":
        return;
      case "job_interrupt": {
        const j = find(args.space).jobs.find((j) => j.job === args.job)!;
        j.status = "failed";
        j.exit_code = 130;
        break;
      }
      case "takeover":
        find(args.space).control = { owner: "human", note: null };
        break;
      case "release":
        find(args.space).control = { owner: "agent", note: null };
        break;
      case "close_space":
        spaces.splice(spaces.indexOf(find(args.space)), 1);
        break;
      case "job_stream": {
        if (scenario === "stream-error") throw { code: "SERVICE_UNAVAILABLE" };
        const found = spaces
          .flatMap((s) => s.jobs)
          .find((j) => j.job === args.job);
        if (!found) throw { code: "NOT_FOUND", message: "会话已关闭" };
        const read = () => {
          const [out, err] = output[args.job].map(bytes);
          const a = out.slice(args.out, args.out + 65_536),
            b = err.slice(args.err, args.err + 65_536);
          return {
            out: a,
            err: b,
            next: [args.out + a.length, args.err + b.length],
            gap: false,
            status: found.status,
            exit_code: found.exit_code,
          };
        };
        for (
          let elapsed = 0;
          elapsed < Math.min(args.waitMs, 5000);
          elapsed += 100
        ) {
          const chunk = read();
          if (
            chunk.out.length ||
            chunk.err.length ||
            found.status !== "running"
          )
            return chunk;
          await sleep(100);
        }
        return read();
      }
      default:
        return;
    }
    void emit("changed");
  },
  { shouldMockEvents: true },
);
