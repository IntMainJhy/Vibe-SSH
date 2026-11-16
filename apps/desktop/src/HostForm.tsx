import { useState } from "react";
import { call, explain, type Host } from "./api";

type Key = { algorithm: string; fingerprint: string };

/** Add or edit a server. Saving requires a fingerprint the user says they compared. */
export function HostForm({ initial, onClose }: { initial?: Host; onClose: () => void }) {
  const [name, setName] = useState(initial?.name ?? "");
  const [address, setAddress] = useState(initial?.host ?? "");
  const [port, setPort] = useState(String(initial?.port ?? 22));
  const [username, setUsername] = useState(initial?.username ?? "root");
  const [useKey, setUseKey] = useState(initial ? initial.identity_file !== null : true);
  const [identity, setIdentity] = useState(initial?.identity_file ?? "~/.ssh/id_ed25519");
  const [secret, setSecret] = useState("");
  const [agentAccess, setAgentAccess] = useState(initial?.agent_access ?? false);
  const [key, setKey] = useState<Key | undefined>(
    initial && { algorithm: "", fingerprint: initial.host_key_sha256 },
  );
  const [verified, setVerified] = useState(Boolean(initial));
  const [busy, setBusy] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string>();

  const portNumber = Number(port);
  const run = (work: Promise<unknown>) => {
    setBusy(true);
    setError(undefined);
    work.catch((e) => setError(explain(e))).finally(() => setBusy(false));
  };
  // A fingerprint only describes the address it was fetched from.
  const retarget = (apply: () => void) => {
    apply();
    setKey(undefined);
    setVerified(false);
  };
  const fetchKey = () =>
    run(
      call<[string, string]>("host_key", { host: address.trim(), port: portNumber }).then(([algorithm, fingerprint]) => {
        setKey({ algorithm, fingerprint });
        setVerified(false);
      }),
    );
  const save = () =>
    run(
      call("save_host", {
        host: {
          id: initial?.id ?? "",
          name: name.trim(),
          host: address.trim(),
          port: portNumber,
          username: username.trim(),
          identity_file: useKey ? identity.trim() : null,
          host_key_sha256: key?.fingerprint ?? "",
          agent_access: agentAccess,
        },
        secret: secret || null,
      }).then(onClose),
    );

  const changed = initial && key && key.fingerprint !== initial.host_key_sha256;
  const ready = name.trim() && address.trim() && username.trim() && portNumber > 0 && key && verified;

  return (
    <div className="scrim">
      <form
        className="dialog host-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (ready) save();
        }}
      >
        <h2>{initial ? "服务器设置" : "添加服务器"}</h2>

        <label>
          名称
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="例如：博客服务器" autoFocus />
        </label>
        <div className="pair">
          <label>
            地址
            <input value={address} onChange={(e) => retarget(() => setAddress(e.target.value))} placeholder="IP 或域名" />
          </label>
          <label className="narrow">
            端口
            <input value={port} onChange={(e) => retarget(() => setPort(e.target.value))} inputMode="numeric" />
          </label>
        </div>
        <label>
          用户名
          <input value={username} onChange={(e) => setUsername(e.target.value)} />
        </label>

        <fieldset>
          <legend>登录方式</legend>
          <label className="choice">
            <input type="radio" checked={useKey} onChange={() => setUseKey(true)} /> 私钥
          </label>
          <label className="choice">
            <input type="radio" checked={!useKey} onChange={() => setUseKey(false)} /> 密码
          </label>
        </fieldset>
        {useKey && (
          <label>
            私钥文件
            <input value={identity} onChange={(e) => setIdentity(e.target.value)} spellCheck={false} />
          </label>
        )}
        <label>
          {useKey ? "私钥口令（没有就留空）" : "密码"}
          <input
            type="password"
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
            placeholder={initial ? "不修改就留空" : ""}
          />
          <small>保存在这台 Mac 的钥匙串里，Agent 读不到。</small>
        </label>

        <div className="trust">
          <div className="trust-head">
            <strong>服务器指纹</strong>
            <button type="button" onClick={fetchKey} disabled={busy || !address.trim() || !(portNumber > 0)}>
              {key ? "重新获取" : "获取指纹"}
            </button>
          </div>
          {key ? (
            <>
              <code>
                {key.algorithm && `${key.algorithm} `}
                {key.fingerprint}
              </code>
              {changed && <p className="warn">和之前保存的指纹不一样。除非你刚重装过这台服务器，否则不要继续。</p>}
              <p>
                在服务器的控制台里运行 <code>ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub</code>
                ，或到云厂商的控制台查看，确认两边一致。指纹不一致说明连到的可能不是你的服务器。
              </p>
              <label className="choice">
                <input type="checkbox" checked={verified} onChange={(e) => setVerified(e.target.checked)} /> 我已核对，指纹一致
              </label>
            </>
          ) : (
            <p>填好地址后获取指纹。之后每次连接都会检查它，不一致就拒绝连接。</p>
          )}
        </div>

        <label className="choice access">
          <input type="checkbox" checked={agentAccess} onChange={(e) => setAgentAccess(e.target.checked)} />
          <span>
            允许 Agent 使用这台服务器
            <small>Agent 将能以 {username || "这个用户"} 的身份在上面运行任何命令。</small>
          </span>
        </label>

        {error && (
          <p className="warn" role="alert">
            {error}
          </p>
        )}
        <div className="actions">
          {initial &&
            (deleting ? (
              <button type="button" className="danger" onClick={() => run(call("delete_host", { id: initial.id }).then(onClose))}>
                确认删除
              </button>
            ) : (
              <button type="button" className="quiet" onClick={() => setDeleting(true)}>
                删除服务器
              </button>
            ))}
          <span className="spacer" />
          <button type="button" onClick={onClose}>
            取消
          </button>
          <button type="submit" className="primary" disabled={busy || !ready}>
            保存
          </button>
        </div>
      </form>
    </div>
  );
}
