import { useRef, useState } from "react";
import { call, explain, type HostDraft } from "./api";
import { Icon } from "./Icon";
import { Modal } from "./Modal";

type Preview = { candidates: HostDraft[]; warnings: { line: number; directive: string }[] };

export function HostImport({ onClose, onSelect }: { onClose: () => void; onSelect: (draft: HostDraft) => void }) {
  const [preview, setPreview] = useState<Preview>();
  const [selected, setSelected] = useState<number>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const choosing = useRef(false);
  const choose = async () => {
    if (choosing.current) return;
    choosing.current = true;
    setBusy(true);
    setError("");
    try {
      const result = await call<Preview | null>("choose_ssh_config");
      if (result) {
        setPreview(result);
        setSelected(undefined);
      }
    } catch (reason) {
      setError(explain(reason));
    } finally {
      choosing.current = false;
      setBusy(false);
    }
  };
  const draft = selected === undefined ? undefined : preview?.candidates[selected];
  return (
    <Modal titleId="import-title" className="host-dialog" onClose={onClose} busy={busy}>
      <section className="host-form import-form">
        <header className="form-heading">
          <span className="server-tile"><Icon name="folder" size={22} /></span>
          <div><h2 id="import-title">导入 OpenSSH 配置</h2><p>选择配置，预览主机，再核对指纹并保存。</p></div>
          <button type="button" className="icon-button" aria-label="关闭导入" disabled={busy} onClick={onClose}><Icon name="close" /></button>
        </header>
        {error && <p className="form-error" role="alert">{error}</p>}
        {preview ? (
          <div className="import-content">
            <fieldset className="import-list" disabled={busy}>
              <legend>{preview.candidates.length} 个主机</legend>
              {!preview.candidates.length && <p>没有找到可直接填写的 Host 别名。</p>}
              {preview.candidates.map((host, index) => (
                <label key={host.name} className="import-row">
                  <input type="radio" name="import-host" checked={selected === index} disabled={host.routed} onChange={() => setSelected(index)} />
                  <span><strong>{host.name}</strong><small>{host.username ? `${host.username}@` : ""}{host.host}:{host.port}</small></span>
                  {host.routed && <small className="import-route">需要代理或跳板机</small>}
                </label>
              ))}
            </fieldset>
            {draft?.issues.map((issue, index) => <p className="muted" key={index}>{issue}</p>)}
            {preview.warnings.length > 0 && <details className="import-warnings"><summary>{preview.warnings.length} 条未导入的指令</summary><ul>{preview.warnings.map((warn) => <li key={warn.line}>第 {warn.line} 行 · <code>{warn.directive}</code></li>)}</ul></details>}
          </div>
        ) : <div className="import-content"><p className="muted">从你选择的文件读取 Host、地址、端口、用户名和私钥路径。</p></div>}
        <footer className="form-actions">
          {preview && <button type="button" disabled={busy} onClick={() => void choose()}>换一个文件</button>}
          <span className="spacer" />
          <button type="button" disabled={busy} onClick={onClose}>取消</button>
          {preview ? <button type="button" className="primary" disabled={busy || !draft || draft.routed} onClick={() => draft && onSelect(draft)}>核对并添加</button> : <button type="button" className="primary" disabled={busy} onClick={() => void choose()}>{busy ? "读取中…" : "选择配置文件"}</button>}
        </footer>
      </section>
    </Modal>
  );
}
