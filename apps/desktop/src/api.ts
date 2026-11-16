import { invoke } from "@tauri-apps/api/core";

export type Host = {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  identity_file: string | null;
  host_key_sha256: string;
  agent_access: boolean;
};

export type JobStatus = "running" | "succeeded" | "failed" | "unknown";

export type Job = {
  job: string;
  command: string | null;
  pty: boolean;
  status: JobStatus;
  exit_code: number | null;
};

export type Space = {
  space: string;
  name: string;
  host_name: string;
  actor: string;
  origin: "agent" | "human";
  connected: boolean;
  control: { owner: "agent" | "human"; note: string | null };
  jobs: Job[];
};

export type Snapshot = {
  hosts: Host[];
  spaces: Space[];
  skill: { skill: boolean; cli: boolean };
};

export type Chunk = {
  out: number[];
  err: number[];
  next: [number, number];
  gap: boolean;
  status: JobStatus;
  exit_code: number | null;
};

export const call = <T = void>(command: string, args?: Record<string, unknown>) =>
  invoke<T>(command, args);

const EXPLAIN: Record<string, string> = {
  HOST_KEY_MISMATCH: "服务器的指纹和你核对过的不一致，已停止连接。请确认服务器没有被替换，再到服务器设置里重新核对指纹。",
  AUTHENTICATION_FAILED: "登录被拒绝。请检查用户名、私钥或密码。",
  CONNECTION_FAILED: "连不上服务器。请检查地址、端口和网络。",
  CONTROL_REVOKED: "这个空间现在由 Agent 操作，接管后才能输入。",
  RESOURCE_LIMIT: "打开的空间或任务太多了，先关闭一些再试。",
};

/** Turns a kernel error into a sentence the user can act on. */
export function explain(error: unknown): string {
  const { code, message } = (error ?? {}) as { code?: string; message?: string };
  return (code && EXPLAIN[code]) || message || String(error);
}
