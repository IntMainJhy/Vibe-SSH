import { useEffect, useRef, useState } from "react";
import { call, explain } from "./api";
import { Icon } from "./Icon";
import { Modal } from "./Modal";

type Rule = { id: string; local_port: number } & (
  | { kind: "local"; target: string; target_port: number }
  | { kind: "dynamic" }
  | { kind: "remote"; remote_port: number }
);

export function Forwards({ space, onClose }: { space: string; onClose: () => void }) {
  const [rules, setRules] = useState<Rule[]>([]);
  const [kind, setKind] = useState<Rule["kind"]>("local");
  const [port, setPort] = useState("");
  const [target, setTarget] = useState("127.0.0.1");
  const [targetPort, setTargetPort] = useState("");
  const [localTarget, setLocalTarget] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const working = useRef(false);
  useEffect(() => {
    let active = true;
    void call<Rule[]>("list_forwards", { space }).then(
      (value) => { if (active && !working.current) setRules(value); },
      (reason) => { if (active) setError(explain(reason)); },
    );
    return () => { active = false; };
  }, [space]);
  const change = async (work: () => Promise<void>) => {
    if (working.current) return;
    working.current = true;
    setBusy(true);
    setError("");
    try { await work(); } catch (reason) { setError(explain(reason)); }
    finally { working.current = false; setBusy(false); }
  };
  return (
    <Modal titleId="forwards-title" className="host-dialog" busy={busy} onClose={onClose}>
      <form className="host-form" onSubmit={(event) => {
        event.preventDefault();
        void change(async () => {
          const bind = { space, localPort: Number(port) };
          const rule = kind === "remote"
            ? await call<Rule>("start_remote_forward", { space, remotePort: Number(port), localPort: Number(localTarget) })
            : kind === "dynamic"
              ? await call<Rule>("start_dynamic_forward", bind)
              : await call<Rule>("start_forward", { ...bind, target: target.trim(), targetPort: Number(targetPort) });
          setRules((old) => [...old.filter((item) => item.id !== rule.id), rule]);
          setPort("");
        });
      }}>
        <header className="form-heading">
          <span className="server-tile"><Icon name="link" /></span>
          <div><h2 id="forwards-title">端口转发</h2><p>转发随当前 SSH 会话关闭。</p></div>
          <button type="button" className="icon-button" aria-label="关闭端口转发" disabled={busy} onClick={onClose}><Icon name="close" /></button>
        </header>
        <fieldset className="form-fields" disabled={busy}>
          <label>
            转发方式
            <select value={kind} onChange={(event) => {
              setKind(event.target.value as Rule["kind"]);
              setPort("");
            }}>
              <option value="local">本地固定目标</option>
              <option value="dynamic">动态 SOCKS5</option>
              <option value="remote">远端端口</option>
            </select>
          </label>
          <label>{kind === "remote" ? "远端监听端口" : "本地监听端口"} <input type="number" min="0" max="65535" placeholder="自动分配" value={port} onChange={(event) => setPort(event.target.value)} /></label>
          {kind === "local" ? <div className="pair">
            <label>远端目标 <input value={target} onChange={(event) => setTarget(event.target.value)} required /></label>
            <label className="narrow">目标端口 <input type="number" min="1" max="65535" value={targetPort} onChange={(event) => setTargetPort(event.target.value)} required /></label>
          </div> : kind === "dynamic" ? <p>本地应用通过回环 SOCKS5 选择远端的 TCP 目标。</p> : <>
            <label>本地目标端口 <input inputMode="numeric" pattern="[0-9]{1,5}" value={localTarget} onChange={(event) => setLocalTarget(event.target.value)} required /></label>
            <p>向 SSH 服务器申请 127.0.0.1 监听，并连接到客户端的 127.0.0.1。服务器的监听范围由 sshd 配置管理。</p>
          </>}
        </fieldset>
        {rules.length > 0 && <div className="import-content"><ul className="forward-list">
          {rules.map((rule) => <li key={rule.id}><code>{rule.kind === "dynamic"
            ? `socks5h://127.0.0.1:${rule.local_port}`
            : rule.kind === "remote"
              ? `远端 :${rule.remote_port} → 本地 :${rule.local_port}`
              : `127.0.0.1:${rule.local_port} → ${rule.target}:${rule.target_port}`}</code><button type="button" disabled={busy} onClick={() => void change(async () => {
            await call("stop_forward", { space, id: rule.id });
            setRules((old) => old.filter((item) => item.id !== rule.id));
          })}>停止</button></li>)}
        </ul></div>}
        {error && <p className="form-error" role="alert">{error}</p>}
        <footer className="form-actions"><span className="spacer" /><button type="button" disabled={busy} onClick={onClose}>完成</button><button type="submit" className="primary" disabled={busy}>{busy ? "处理中…" : "添加转发"}</button></footer>
      </form>
    </Modal>
  );
}
