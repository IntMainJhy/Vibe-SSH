import { useEffect, useMemo, useRef, useState } from "react";
import { call, explain } from "./api";
import { Icon } from "./Icon";
import { Modal } from "./Modal";
import { bytes } from "./Transfers";
import { openText, saveText, type TextFormat } from "./text-format";

type Document = {
  path: string;
  text: string;
  revision: string;
  save_supported: boolean;
};
type Saved = { document: Document; backup: string | null };
export function TextEditor({
  server,
  space,
  path,
  writable,
  onClose,
  onSaved,
}: {
  server: string;
  space: string;
  path: string;
  writable: boolean;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [document, setDocument] = useState<Document>();
  const [value, setValue] = useState("");
  const [format, setFormat] = useState<TextFormat>({
    bom: false,
    ending: "LF",
    mixed: false,
  });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [copyFeedback, setCopyFeedback] = useState("");
  const [backup, setBackup] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<"close" | "reload">();
  const [request, setRequest] = useState(0);
  const current = useRef<Document | undefined>(undefined);
  const alive = useRef(true);
  const textarea = useRef<HTMLTextAreaElement>(null);
  const wasConfirm = useRef(false);
  useEffect(() => {
    if (!confirm && wasConfirm.current) textarea.current?.focus();
    wasConfirm.current = !!confirm;
  }, [confirm]);
  current.current = document;
  const wire = useMemo(
    () => (format.mixed ? (document?.text ?? "") : saveText(value, format)),
    [value, format, document?.text],
  );
  const length = useMemo(() => new TextEncoder().encode(wire).length, [wire]);
  const dirty = !!document && wire !== document.text;
  const editable = writable && document?.save_supported && !format.mixed;
  const busy = loading || saving;
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      const doc = current.current;
      if (doc)
        void call("forget_text", { space, revision: doc.revision }).catch(
          () => {},
        );
    };
  }, [space]);
  useEffect(() => {
    let active = true;
    setLoading(true);
    setError("");
    setMessage("");
    void call<Document>("text_file", { space, path })
      .then((doc) => {
        if (!active) {
          void call("forget_text", { space, revision: doc.revision }).catch(
            () => {},
          );
          return;
        }
        const previous = current.current;
        const parsed = openText(doc.text);
        current.current = doc;
        setDocument(doc);
        setValue(parsed.value);
        setFormat(parsed.format);
        setBackup(null);
        if (previous)
          void call("forget_text", {
            space,
            revision: previous.revision,
          }).catch(() => {});
      })
      .catch((e) => {
        if (active) setError(explain(e));
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [space, path, request]);
  const leave = () => {
    if (saving) return;
    if (confirm) {
      setConfirm(undefined);
      return;
    }
    if (dirty) {
      setCopyFeedback("");
      setConfirm("close");
    } else onClose();
  };
  const reload = () => {
    if (dirty) {
      setCopyFeedback("");
      setConfirm("reload");
    } else setRequest((n) => n + 1);
  };
  const save = async () => {
    if (!document || busy || !editable || !dirty || length > 1_048_576) return;
    setSaving(true);
    setError("");
    setMessage("");
    try {
      const result = await call<Saved>("save_text", {
        space,
        revision: document.revision,
        text: wire,
      });
      if (!alive.current) return;
      const parsed = openText(result.document.text);
      current.current = result.document;
      setDocument(result.document);
      setValue(parsed.value);
      setFormat(parsed.format);
      setBackup(result.backup);
      setMessage("已保存到服务器");
      onSaved();
    } catch (e) {
      if (alive.current) setError(explain(e));
    } finally {
      if (alive.current) setSaving(false);
    }
  };
  const copy = async () => {
    try {
      await call("copy_text", { text: wire });
      setMessage("草稿已复制");
      setCopyFeedback("草稿已复制");
    } catch (e) {
      setError(explain(e));
      setCopyFeedback(explain(e));
    }
  };
  return (
    <Modal
      titleId="text-editor-title"
      busy={saving}
      onClose={leave}
      className="text-editor-dialog"
    >
      <div className="editor-content" inert={confirm ? true : undefined}>
        <div className="text-editor-header">
          <div>
            <h2 id="text-editor-title">
              <Icon name="file" size={18} />
              <span className="editor-filename" title={path.split("/").at(-1)}>
                {path.split("/").at(-1)}
              </span>
              {dirty && <span className="editor-unsaved">未保存</span>}
            </h2>
            <p>
              <span className="editor-server" title={server}>
                {server}
              </span>
              <span>·</span>
              <span className="editor-path" title={path}>
                {path}
              </span>
            </p>
          </div>
          <button
            className="icon-button"
            aria-label="关闭文本编辑器"
            disabled={saving}
            onClick={leave}
          >
            <Icon name="close" />
          </button>
        </div>
        <div className="text-editor-toolbar">
          <span>
            {loading || !document
              ? "读取文本"
              : !writable
                ? "只读预览 · 接管空间后可编辑"
                : !document.save_supported
                  ? "此服务器仅支持预览"
                  : format.mixed
                    ? "混合换行 · 仅支持预览"
                    : saving
                      ? "正在保存…"
                      : dirty
                        ? "有未保存的修改"
                        : "未修改"}
          </span>
          <div>
            <button
              disabled={!document || busy || length > 1_048_576}
              onClick={() => void copy()}
            >
              <Icon name="copy" size={14} />
              复制草稿
            </button>
            <button disabled={busy} onClick={reload}>
              <Icon name="refresh" size={14} />
              重新读取
            </button>
          </div>
        </div>
        {error && (
          <div className="editor-error" role="alert">
            {error}
          </div>
        )}
        {message && (
          <div className="editor-message" role="status">
            {message}
            {backup && (
              <span className="editor-backup" title={backup}>
                {" "}
                · 原文件备份：{backup}
              </span>
            )}
          </div>
        )}
        {loading ? (
          <div className="editor-loading">正在读取文本…</div>
        ) : !document ? (
          <div className="editor-loading">
            <p>无法预览这个文件</p>
            <button onClick={() => setRequest((n) => n + 1)}>重新读取</button>
          </div>
        ) : (
          <textarea
            ref={textarea}
            autoFocus
            className="text-editor-input"
            aria-label="远端文本内容"
            value={value}
            readOnly={!editable || saving}
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            onChange={(e) => {
              setValue(e.target.value);
              setMessage("");
            }}
            onKeyDown={(e) => {
              if (
                (navigator.platform.startsWith("Mac")
                  ? e.metaKey
                  : e.ctrlKey) &&
                e.key.toLowerCase() === "s"
              ) {
                e.preventDefault();
                void save();
              }
            }}
          />
        )}
        <footer className="text-editor-footer">
          <span>
            {bytes(length)} / 1 MiB · UTF-8{format.bom ? " BOM" : ""} ·{" "}
            {format.mixed ? "混合换行" : format.ending}
            {length > 1_048_576 && " · 超出保存上限"}
          </span>
          <div className="actions">
            <button disabled={saving} onClick={leave}>
              关闭
            </button>
            <button
              className="primary"
              disabled={busy || !editable || !dirty || length > 1_048_576}
              onClick={() => void save()}
            >
              {saving ? "保存中…" : "保存到服务器"}
            </button>
          </div>
        </footer>
      </div>
      {confirm && (
        <div
          className="editor-confirm"
          role="alertdialog"
          aria-labelledby="editor-confirm-title"
        >
          <h3 id="editor-confirm-title">
            {confirm === "close"
              ? "放弃未保存的修改？"
              : "重新读取会替换当前草稿"}
          </h3>
          <p className="editor-confirm-context">
            <strong>{server}</strong>
            <code>{path}</code>
          </p>
          <p>
            {confirm === "close"
              ? "未保存的修改将被丢弃，服务器文件保持原样。"
              : "未保存的修改将被丢弃，并重新读取服务器文件。"}
          </p>
          {copyFeedback && <p role="status">{copyFeedback}</p>}
          <div className="actions">
            <button disabled={length > 1_048_576} onClick={() => void copy()}>
              <Icon name="copy" size={14} />
              复制草稿
            </button>
            <button autoFocus onClick={() => setConfirm(undefined)}>
              继续编辑
            </button>
            <button
              className="primary"
              onClick={() => {
                if (confirm === "close") onClose();
                else {
                  setConfirm(undefined);
                  setRequest((n) => n + 1);
                }
              }}
            >
              {confirm === "close" ? "放弃并关闭" : "丢弃草稿并重新读取"}
            </button>
          </div>
        </div>
      )}
    </Modal>
  );
}
