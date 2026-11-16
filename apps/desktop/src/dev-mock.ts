// Sample data for developing the interface in a plain browser (`npm run dev`), where the
// app shell is absent. Loaded only in development; it is not part of the production bundle.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { Host, Snapshot, Space } from "./api";

const key = "SHA256:nThbg6kXUpJWGl7E1IGOCspRomTxdCARLviKw6E5SY8";
const hosts: Host[] = [
  { id: "blog", name: "博客服务器", host: "203.0.113.10", port: 22, username: "deploy", identity_file: "/Users/me/.ssh/id_ed25519", host_key_sha256: key, agent_access: true },
  { id: "db", name: "数据库", host: "203.0.113.20", port: 2222, username: "root", identity_file: null, host_key_sha256: key, agent_access: false },
];
const job = (id: string, command: string | null, status: Space["jobs"][0]["status"], pty = false) => ({
  job: id, command, pty, status, exit_code: status === "succeeded" ? 0 : status === "failed" ? 1 : null,
});
const spaces: Space[] = [
  {
    space: "s1", name: "部署博客 v2", host_name: "博客服务器", actor: "Claude Code", origin: "agent", connected: true,
    control: { owner: "agent", note: null },
    jobs: [job("j1", "npm ci && npm run build", "succeeded"), job("j2", "nginx -t", "failed"), job("j3", "docker compose up --build", "running")],
  },
  {
    space: "s2", name: "给站点续期证书", host_name: "博客服务器", actor: "Codex", origin: "agent", connected: true,
    control: { owner: "human", note: "sudo 需要你的密码，请在终端里输入" },
    jobs: [job("j4", "sudo certbot renew", "running", true)],
  },
  { space: "s3", name: "终端", host_name: "数据库", actor: "me", origin: "human", connected: true, control: { owner: "human", note: null }, jobs: [job("j5", null, "running", true)] },
];
const output: Record<string, [string, string]> = {
  j1: ["added 312 packages in 9s\n\n> blog@2.0.0 build\n> vite build\n\n✓ 148 modules transformed.\ndist/index.html   0.62 kB\n✓ built in 2.31s\n", ""],
  j2: ["", "nginx: [emerg] unknown directive \"servr_name\" in /etc/nginx/sites-enabled/blog:4\nnginx: configuration file /etc/nginx/nginx.conf test failed\n"],
  j3: ["[+] Building 12.4s (9/14)\n => [web 3/6] COPY package*.json ./\n => [web 4/6] RUN npm ci --omit=dev\n", ""],
  j4: ["[sudo] password for deploy: ", ""],
  j5: ["Last login: Mon Nov 16 21:04:11 2026 from 198.51.100.7\r\nroot@db:~# ", ""],
};
const bytes = (text: string) => Array.from(new TextEncoder().encode(text));
const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms));
const find = (id: string) => spaces.find((s) => s.space === id)!;

mockWindows("main");
mockIPC(
  async (command, payload) => {
    const args = (payload ?? {}) as Record<string, any>;
    switch (command) {
      case "snapshot":
        return { hosts, spaces, skill: { skill: false, cli: false } } satisfies Snapshot;
      case "host_key":
        await sleep(400);
        return ["ssh-ed25519", key];
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
        const found = spaces.flatMap((s) => s.jobs).find((j) => j.job === args.job)!;
        const [out, err] = args.out + args.err === 0 ? output[args.job] : ["", ""];
        if (args.out + args.err > 0) await sleep(args.waitMs);
        const sent = output[args.job].map((text) => bytes(text).length);
        return { out: bytes(out), err: bytes(err), next: sent, gap: false, status: found.status, exit_code: found.exit_code };
      }
      default:
        return;
    }
    void emit("changed");
  },
  { shouldMockEvents: true },
);
