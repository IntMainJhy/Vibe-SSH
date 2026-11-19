import { useRef, useState } from "react";
import type { Host, Space } from "./api";
import { Icon } from "./Icon";
import { Modal } from "./Modal";

/** Navigation is committed only after choosing a result; Escape preserves the workspace. */
export function QuickSwitch({ hosts, spaces, onSelect, onConnect, onClose }: {
  hosts: Host[];
  spaces: Space[];
  onSelect: (id: string) => void;
  onConnect: (host: Host) => Promise<void>;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const searchInput = useRef<HTMLInputElement>(null);
  const results = useRef<HTMLDivElement>(null);
  const needle = query.trim().toLocaleLowerCase();
  const matches = (...values: string[]) => values.join(" ").toLocaleLowerCase().includes(needle);
  const sessions = spaces.filter(s => matches(s.name, s.host_name, s.actor));
  const servers = hosts.filter(h => matches(h.name, h.host, h.username));
  const open = (id: string) => { onClose(); onSelect(id); };

  return (
    <Modal titleId="quick-switch-title" className="quick-switch" onClose={onClose} initialFocus={searchInput}>
      <header className="quick-switch-heading">
        <h2 id="quick-switch-title">搜索与切换</h2>
        <button className="icon-button" aria-label="关闭搜索" onClick={onClose}><Icon name="close" size={17} /></button>
      </header>
      <div className="quick-switch-input">
        <Icon name="search" size={20} />
        <input ref={searchInput} aria-label="搜索服务器或会话" placeholder="输入服务器、地址或会话名称…" value={query}
          onChange={event => setQuery(event.target.value)}
          onKeyDown={event => {
            if (event.key === "ArrowDown" || event.key === "Enter") {
              event.preventDefault();
              const first = results.current?.querySelector<HTMLButtonElement>("button");
              if (event.key === "Enter") first?.click(); else first?.focus();
            }
          }} />
        <kbd>esc</kbd>
      </div>
      <div className="quick-switch-results" ref={results} onKeyDown={event => {
        if (!["ArrowDown", "ArrowUp"].includes(event.key)) return;
        event.preventDefault();
        const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button")];
        const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
        buttons[(index + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length]?.focus();
      }}>
        {sessions.length > 0 && <section aria-label="匹配的会话">
          <h3>已有会话 <span>{sessions.length}</span></h3>
          {sessions.map(s => <button className="quick-result" key={s.space} onClick={() => open(s.space)}>
            <Icon name={s.origin === "agent" ? "link" : "terminal"} size={18} />
            <span><strong>{s.origin === "human" ? s.host_name : s.name}</strong><small>{s.origin === "agent" ? `${s.host_name} · ${s.actor}` : "个人终端"}{!s.connected ? " · 已断开" : s.control.note ? " · 等待你处理" : ""}</small></span>
            <span className="quick-result-action">打开会话 <Icon name="arrow" size={14} /></span>
          </button>)}
        </section>}
        {servers.length > 0 && <section aria-label="匹配的服务器">
          <h3>服务器 <span>{servers.length}</span></h3>
          {servers.map(h => <button className="quick-result" key={h.id} onClick={() => { onClose(); void onConnect(h); }}>
            <Icon name="server" size={18} /><span><strong>{h.name}</strong><small>{h.username}@{h.host}:{h.port}</small></span>
            <span className="quick-result-action">新建连接 <Icon name="plus" size={14} /></span>
          </button>)}
        </section>}
        {!sessions.length && !servers.length && <div className="quick-switch-empty"><Icon name="search" size={26} /><p>没有找到匹配项</p><small>试试名称、IP 地址或登录用户名。</small></div>}
      </div>
      <footer><span>↑ ↓ 选择 · Enter 打开</span><span>Esc 返回当前工作</span></footer>
    </Modal>
  );
}
