// Sample data for developing the interface in a plain browser (`npm run dev`), where the
// app shell is absent. Loaded only in development; it is not part of the production bundle.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { Host, Snapshot, Space } from "./api";

const key = "SHA256:nThbg6kXUpJWGl7E1IGOCspRomTxdCARLviKw6E5SY8";
let installed = false;
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
mockIPC(
  async (command, payload) => {
    const args = (payload ?? {}) as Record<string, any>;
    switch (command) {
      case "snapshot":
        if (scenario === "offline")
          throw { code: "SERVICE_UNAVAILABLE", message: "preview unavailable" };
        return structuredClone({
          status: {
            version: "0.1.0",
            protocol_version: 2,
            instance: "preview",
          },
          hosts,
          spaces,
          skill: { skill: installed, cli: installed },
        } satisfies Snapshot);
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
      case "install_skill":
        installed = true;
        await emit("changed");
        return { linked: ["preview"], skipped: [] };
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
