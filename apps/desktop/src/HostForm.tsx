import { useRef, useState } from "react";
import { call, explain, type Host, type HostDraft, type ProxySettings } from "./api";
import { Icon } from "./Icon";
import { Modal } from "./Modal";

type Key = { algorithm: string; fingerprint: string };

export function HostForm({
  initial,
  draft,
  hosts,
  onClose,
}: {
  initial?: Host;
  draft?: HostDraft;
  hosts: Host[];
  onClose: () => void;
}) {
  const [name, setName] = useState(initial?.name ?? draft?.name ?? "");
  const [address, setAddress] = useState(initial?.host ?? draft?.host ?? "");
  const [port, setPort] = useState(String(initial?.port ?? draft?.port ?? 22));
  const [username, setUsername] = useState(initial?.username ?? (draft ? draft.username ?? "" : "root"));
  const [auth, setAuth] = useState(initial?.ssh_agent ? "agent" : initial && !initial.identity_file ? "password" : "key");
  const [jump, setJump] = useState(initial?.jump_host ?? "");
  const [route, setRoute] = useState<"direct" | "jump" | ProxySettings["kind"]>(initial?.proxy?.kind ?? (initial?.jump_host ? "jump" : "direct"));
  const [proxyAddress, setProxyAddress] = useState(initial?.proxy?.address ?? "");
  const [proxyPort, setProxyPort] = useState(initial?.proxy ? String(initial.proxy.port) : "");
  const jumpHosts = hosts.filter((host) => host.id !== initial?.id);
  const proxy = route === "http_connect" || route === "socks5" ? { kind: route, address: proxyAddress.trim(), port: Number(proxyPort) } : undefined;
  const validRoute = route === "jump" ? jumpHosts.some((host) => host.id === jump)
    : !proxy || (proxy.address && Number.isInteger(proxy.port) && proxy.port > 0 && proxy.port <= 65535);
  const useKey = auth === "key";
  const [identity, setIdentity] = useState(
    initial?.identity_file ?? draft?.identity_file ?? "~/.ssh/id_ed25519",
  );
  const [secret, setSecret] = useState("");
  const [agentAccess, setAgentAccess] = useState(
    initial?.agent_access ?? false,
  );
  const [key, setKey] = useState<Key | undefined>(
    initial && { algorithm: "", fingerprint: initial.host_key_sha256 },
  );
  const [verified, setVerified] = useState(Boolean(initial));
  const [busy, setBusy] = useState<"probe" | "save" | "delete">();
  const working = useRef(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string>();
  const portNumber = Number(port);
  const validPort =
    Number.isInteger(portNumber) && portNumber >= 1 && portNumber <= 65535;
  const requiresPassword =
    auth === "password" && (!initial || initial.identity_file !== null || initial.ssh_agent);
  const ready = Boolean(
    name.trim() &&
      address.trim() &&
      username.trim() &&
      validPort &&
      validRoute &&
      key &&
      verified &&
      (!useKey || identity.trim()) &&
      (!requiresPassword || secret),
  );

  const run = async (
    operation: NonNullable<typeof busy>,
    work: () => Promise<unknown>,
  ) => {
    if (working.current) return;
    working.current = true;
    setBusy(operation);
    setError(undefined);
    try {
      await work();
    } catch (e) {
      setError(explain(e));
    } finally {
      working.current = false;
      setBusy(undefined);
    }
  };
  const retarget = (apply: () => void) => {
    apply();
    setKey(undefined);
    setVerified(false);
  };
  const fetchKey = () => {
    if (!address.trim() || !validPort || !validRoute) return;
    void run("probe", async () => {
      const [algorithm, fingerprint] = await call<[string, string]>(
        "host_key",
        { host: address.trim(), port: portNumber, jumpHost: route === "jump" ? jump : null, proxy: proxy ?? null },
      );
      setKey({ algorithm, fingerprint });
      setVerified(false);
    });
  };
  const save = () => {
    if (!ready) return;
    void run("save", async () => {
      await call("save_host", {
        host: {
          id: initial?.id ?? "",
          name: name.trim(),
          host: address.trim(),
          port: portNumber,
          username: username.trim(),
          identity_file: useKey ? identity.trim() : null,
          ssh_agent: auth === "agent" || undefined,
          jump_host: route === "jump" ? jump : undefined,
          proxy,
          host_key_sha256: key?.fingerprint ?? "",
          agent_access: agentAccess,
        },
        secret: auth === "agent" ? null : secret || null,
      });
      onClose();
    });
  };

  return (
    <Modal
      titleId="host-title"
      className="host-dialog"
      onClose={onClose}
      busy={Boolean(busy)}
    >
      <form
        className="host-form"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <header className="form-heading">
          <span className="server-tile">
            <Icon name="server" size={23} />
          </span>
          <div>
            <h2 id="host-title">{initial ? "服务器设置" : "添加服务器"}</h2>
            <p>保存连接资料，核对服务器身份后开始使用。</p>
          </div>
          <button
            type="button"
            className="icon-button"
            disabled={Boolean(busy)}
            aria-label="关闭服务器设置"
            onClick={onClose}
          >
            <Icon name="close" />
          </button>
        </header>
        <fieldset className="form-fields" disabled={Boolean(busy)}>
          <label>
            服务器名称
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="例如：生产服务器"
              autoFocus
              maxLength={80}
              required
            />
          </label>
          <div className="pair">
            <label>
              主机地址
              <input
                value={address}
                onChange={(e) => retarget(() => setAddress(e.target.value))}
                placeholder="IP 地址或域名"
                spellCheck={false}
                required
              />
            </label>
            <label className="narrow">
              端口
              <input
                value={port}
                onChange={(e) => retarget(() => setPort(e.target.value))}
                type="number"
                min="1"
                max="65535"
                step="1"
                required
                aria-invalid={Boolean(port) && !validPort}
              />
            </label>
          </div>
          <label>
            登录用户
            <input
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              autoComplete="username"
              spellCheck={false}
              required
            />
          </label>
          <div className="form-divider" />
          <label>
            连接路径
            <select value={route} onChange={(e) => retarget(() => setRoute(e.target.value as typeof route))}>
              <option value="direct">直连</option>
              <option value="jump" disabled={!jumpHosts.length && route !== "jump"}>SSH 跳板机</option>
              <option value="http_connect">HTTP CONNECT 代理</option>
              <option value="socks5">SOCKS5 代理</option>
            </select>
          </label>
          {route === "jump" && (
            <label>
              跳板机
              <select value={jump} onChange={(e) => retarget(() => setJump(e.target.value))}>
                <option value="">选择已保存的服务器</option>
                {jump && !validRoute && <option value={jump}>跳板机不可用</option>}
                {jumpHosts.map((host) => <option value={host.id} key={host.id}>经 {host.name} 连接</option>)}
              </select>
            </label>
          )}
          {proxy && <div className="pair">
            <label>代理地址 <input value={proxyAddress} required spellCheck={false}
              onChange={(e) => retarget(() => setProxyAddress(e.target.value))} /></label>
            <label className="narrow">代理端口 <input type="number" value={proxyPort} min={1} max={65535} required
              onChange={(e) => retarget(() => setProxyPort(e.target.value))} /></label>
          </div>}
          <fieldset className="auth-choice">
            <legend>认证方式</legend>
            <label>
              <input
                type="radio"
                name="auth"
                checked={useKey}
                onChange={() => setAuth("key")}
              />
              SSH 私钥
            </label>
            <label>
              <input
                type="radio"
                name="auth"
                checked={auth === "password"}
                onChange={() => setAuth("password")}
              />
              密码登录
            </label>
            <label>
              <input type="radio" name="auth" checked={auth === "agent"} onChange={() => setAuth("agent")} />
              SSH 密钥代理
            </label>
          </fieldset>
          {useKey && (
            <label>
              私钥文件
              <input
                value={identity}
                onChange={(e) => setIdentity(e.target.value)}
                spellCheck={false}
                placeholder="~/.ssh/id_ed25519"
                required
              />
            </label>
          )}
          {auth !== "agent" ? <label>
            {useKey ? "私钥口令" : "登录密码"}
            <input
              type="password"
              value={secret}
              onChange={(e) => setSecret(e.target.value)}
              autoComplete="new-password"
              placeholder={
                initial
                  ? "留空保留原有凭据"
                  : useKey
                    ? "未加密的私钥可留空"
                    : "输入登录密码"
              }
              required={requiresPassword}
            />
            <small>
              <Icon name="shield" size={12} />
              保存到系统钥匙串，Agent 接口不提供凭据。
            </small>
          </label> : <p className="muted">使用系统 SSH 密钥代理中已加载的密钥。</p>}
          <div className="trust">
            <div className="trust-head">
              <strong>
                <Icon name="shield" size={16} />
                服务器身份
              </strong>
              <button
                type="button"
                onClick={fetchKey}
                disabled={!address.trim() || !validPort || !validRoute}
              >
                {busy === "probe"
                  ? "正在获取…"
                  : key
                    ? "重新获取指纹"
                    : "获取指纹"}
              </button>
            </div>
            {key ? (
              <>
                <code className="fingerprint">
                  {key.algorithm && <span>{key.algorithm}</span>}
                  {key.fingerprint}
                </code>
                {initial && key.fingerprint !== initial.host_key_sha256 && (
                  <p className="warn">
                    指纹已变化。请先确认服务器是否重装或更换。
                  </p>
                )}
                <p>请与服务器或云厂商控制台中的主机公钥指纹独立比对。</p>
                <label className="choice">
                  <input
                    type="checkbox"
                    checked={verified}
                    onChange={(e) => setVerified(e.target.checked)}
                  />
                  我已核对，指纹一致
                </label>
              </>
            ) : (
              <p>获取指纹并核对后才能保存。后续连接会自动检查服务器身份。</p>
            )}
          </div>
          <label className="access choice">
            <input
              type="checkbox"
              checked={agentAccess}
              onChange={(e) => setAgentAccess(e.target.checked)}
            />
            <span>
              <strong>允许 Agent 操作</strong>
              <small>
                通过 Skill 以 {username || "此用户"}{" "}
                的权限运行命令。关闭并保存后，已有 Agent 会话由你控制，运行中的命令可查看或手动中断。
              </small>
            </span>
          </label>
        </fieldset>
        {error && (
          <p className="form-error" role="alert">
            <Icon name="alert" size={16} />
            {error}
          </p>
        )}
        {deleting && (
          <p className="form-error" role="alert">
            删除连接资料会撤销 Agent 接入。已打开的会话仍可查看和关闭，服务器上的文件不受影响。
          </p>
        )}
        <footer className="form-actions">
          {initial && (
            <button
              type="button"
              className={deleting ? "danger" : "text-button danger-text"}
              disabled={Boolean(busy)}
              onClick={() =>
                deleting
                  ? void run("delete", async () => {
                      await call("delete_host", { id: initial.id });
                      onClose();
                    })
                  : setDeleting(true)
              }
            >
              {busy === "delete"
                ? "删除中…"
                : deleting
                  ? "确认删除资料"
                  : "删除服务器"}
            </button>
          )}
          <span className="spacer" />
          <button type="button" disabled={Boolean(busy)} onClick={onClose}>
            取消
          </button>
          <button
            type="submit"
            className="primary"
            disabled={Boolean(busy) || !ready}
          >
            {busy === "save" ? "保存中…" : "保存服务器"}
          </button>
        </footer>
      </form>
    </Modal>
  );
}
