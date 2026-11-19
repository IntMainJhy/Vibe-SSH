import { FitAddon } from "@xterm/addon-fit";
import type { SearchAddon } from "@xterm/addon-search";
import { Terminal as Xterm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef, useState } from "react";
import { call, explain, report, type Chunk } from "./api";
import { Icon } from "./Icon";
import { inputFrames, inputQueue } from "./terminal-input";

const ENDED = {
  succeeded: "已结束",
  failed: "已失败",
  unknown: "结果未知，请到服务器上核实",
};
type Props = {
  space: string;
  job: string;
  pty: boolean;
  writable: boolean;
  fontSize: number;
  onFontSize: (size: number) => void;
  onCopyCommand?: () => void;
  onError: (error: string) => void;
};

/** Output is replayed on attachment, but only fresh user input is ever sent to the server. */
export function Terminal({
  space,
  job,
  pty,
  writable,
  fontSize,
  onFontSize,
  onCopyCommand,
  onError,
}: Props) {
  const box = useRef<HTMLDivElement>(null);
  const term = useRef<Xterm | null>(null);
  const fitter = useRef<FitAddon | null>(null);
  const finder = useRef<SearchAddon | null>(null);
  const findInput = useRef<HTMLInputElement>(null);
  const findRequest = useRef(0);
  const canWrite = useRef(writable);
  const generation = useRef(0);
  const onProblem = useRef(onError);
  const currentFont = useRef(fontSize);
  const [problem, setProblem] = useState<string>();
  const [attachment, setAttachment] = useState(0);
  const [ready, setReady] = useState(false);
  const [finding, setFinding] = useState(false);
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<boolean>();
  const [exporting, setExporting] = useState(false);
  // Gates change during render, so an already queued callback cannot use stale ownership.
  const enabled = writable && !problem;
  if (canWrite.current !== enabled) generation.current += 1;
  canWrite.current = enabled;
  onProblem.current = onError;
  currentFont.current = fontSize;

  useEffect(() => {
    const xterm = new Xterm({
      convertEol: !pty,
      fontFamily: '"SF Mono", ui-monospace, Menlo, monospace',
      fontSize: currentFont.current,
      scrollback: 5000,
      disableStdin: true,
      cursorBlink: canWrite.current,
      theme: {
        background: "#111111",
        foreground: "#e5e5e5",
        cursor: "#eeeeee",
        selectionBackground: "#444444",
        selectionInactiveBackground: "#616161",
      },
    });
    const fit = new FitAddon();
    xterm.loadAddon(fit);
    xterm.open(box.current!);
    term.current = xterm;
    fitter.current = fit;
    let alive = true,
      live = false,
      failed = false;
    const fail = (error: unknown) => {
      if (!alive || failed) return;
      failed = true;
      xterm.options.disableStdin = true;
      const message = explain(error);
      setProblem(message);
      onProblem.current(message);
    };
    const enqueue = inputQueue(
      () => alive && canWrite.current && !failed,
      fail,
      () => generation.current,
    );
    const send = (command: string, args: Record<string, unknown>) =>
      enqueue(() => call(command, { space, job, ...args }));
    const typed = xterm.onData((data) => {
      if (!live || !canWrite.current || failed) return;
      for (const frame of inputFrames(data))
        void send("job_input", { data: frame });
    });
    const resized = xterm.onResize(({ cols, rows }) => {
      if (pty && canWrite.current)
        void send("job_resize", { columns: cols, rows });
    });
    const refit = () => {
      if (box.current?.clientWidth && box.current.clientHeight) fit.fit();
    };
    refit();
    const observer = new ResizeObserver(refit);
    observer.observe(box.current!);
    // Separate decoders prevent stdout and stderr fragments from corrupting one another.
    const stdout = new TextDecoder(),
      stderr = new TextDecoder();
    const write = (text: string) =>
      new Promise<void>((done) => {
        if (alive && text) xterm.write(text, done);
        else done();
      });
    void (async () => {
      let next: [number, number] = [0, 0];
      while (alive) {
        let chunk: Chunk;
        try {
          chunk = await call<Chunk>("job_stream", {
            job,
            out: next[0],
            err: next[1],
            waitMs: live ? 25_000 : 0,
          });
        } catch (error) {
          fail(error);
          return;
        }
        if (!alive) return;
        if (chunk.gap) {
          stdout.decode();
          stderr.decode();
          await write("\r\n\x1b[2m[更早的输出已被清理]\x1b[0m\r\n");
        }
        // Raw output is data. Do not infer success from what is printed in the terminal.
        await write(
          stderr.decode(Uint8Array.from(chunk.err), { stream: true }),
        );
        await write(
          stdout.decode(Uint8Array.from(chunk.out), { stream: true }),
        );
        next = chunk.next;
        if (!live && chunk.out.length < 65_536 && chunk.err.length < 65_536) {
          live = true;
          setReady(true);
          if (canWrite.current) xterm.focus();
          report(
            `terminal ${xterm.cols}x${xterm.rows} in ${box.current?.clientWidth}px`,
          );
        }
        if (
          chunk.status !== "running" &&
          chunk.out.length + chunk.err.length === 0
        ) {
          await write(stderr.decode() + stdout.decode());
          const code =
            chunk.exit_code === null ? "" : `，退出码 ${chunk.exit_code}`;
          await write(`\r\n\x1b[2m[${ENDED[chunk.status]}${code}]\x1b[0m\r\n`);
          return;
        }
      }
    })();
    return () => {
      alive = false;
      typed.dispose();
      resized.dispose();
      observer.disconnect();
      xterm.dispose();
      term.current = null;
      fitter.current = null;
      finder.current = null;
    };
  }, [space, job, pty, attachment]);

  useEffect(() => {
    const xterm = term.current;
    if (!xterm || problem) return;
    xterm.options.disableStdin = !writable || !ready;
    xterm.options.cursorBlink = writable;
    if (writable) {
      xterm.focus();
      if (pty)
        void call("job_resize", {
          space,
          job,
          columns: xterm.cols,
          rows: xterm.rows,
        }).catch((e) => {
          setProblem(explain(e));
          onProblem.current(explain(e));
        });
    }
  }, [writable, space, job, pty, problem, ready]);
  useEffect(() => {
    if (!term.current) return;
    term.current.options.fontSize = fontSize;
    fitter.current?.fit();
  }, [fontSize]);

  const openFind = () => {
    setFinding(true);
    requestAnimationFrame(() => {
      findInput.current?.focus();
      findInput.current?.select();
    });
  };
  const closeFind = () => {
    findRequest.current += 1;
    finder.current?.clearDecorations();
    setFinding(false);
    setFound(undefined);
    term.current?.focus();
  };
  const find = async (backwards = false) => {
    const xterm = term.current;
    if (!xterm || !query) return;
    const request = ++findRequest.current;
    try {
      if (!finder.current) {
        const { SearchAddon } = await import("@xterm/addon-search");
        if (term.current !== xterm || request !== findRequest.current) return;
        finder.current = new SearchAddon();
        xterm.loadAddon(finder.current);
      }
      setFound(backwards ? finder.current.findPrevious(query) : finder.current.findNext(query));
    } catch (error) {
      onProblem.current(explain(error));
    }
  };
  const exportText = async () => {
    const buffer = term.current?.buffer.active;
    if (!buffer || exporting) return;
    const lines = [];
    let logical = "";
    for (let i = 0; i < buffer.length; i++) {
      const nextWrapped = buffer.getLine(i + 1)?.isWrapped;
      logical += buffer.getLine(i)?.translateToString(!nextWrapped) ?? "";
      if (!nextWrapped) {
        lines.push(logical);
        logical = "";
      }
    }
    while (lines.at(-1) === "") lines.pop();
    setExporting(true);
    try {
      await call("export_terminal", { text: lines.join("\n") + "\n" });
    } catch (error) {
      onProblem.current(explain(error));
    } finally {
      setExporting(false);
    }
  };

  return (
    <div className="terminal-content" onKeyDownCapture={(event) => {
      if (event.metaKey && event.key.toLowerCase() === "f") {
        event.preventDefault();
        event.stopPropagation();
        openFind();
      }
    }}>
      <div className="terminal-tools">
        <span><Icon name="terminal" size={14} />{pty ? "交互终端" : "命令输出"}</span>
        <div>
          <button aria-label="缩小终端字号" disabled={fontSize <= 11} onClick={() => onFontSize(fontSize - 1)}>A−</button>
          <span>{fontSize}px</span>
          <button aria-label="放大终端字号" disabled={fontSize >= 20} onClick={() => onFontSize(fontSize + 1)}>A+</button>
          <i />
          <button aria-expanded={finding} title="查找终端文本（⌘F）" onClick={openFind}>
            <Icon name="search" size={14} />查找
          </button>
          <button title="保存终端保留的文本" disabled={!ready || exporting} onClick={() => void exportText()}>
            <Icon name="download" size={14} />导出文本
          </button>
          <button onClick={() => term.current?.clear()}>清空显示</button>
          {onCopyCommand && <button title="复制完整命令" onClick={onCopyCommand}>
            <Icon name="copy" size={14} />复制命令
          </button>}
        </div>
      </div>
      {finding && (
        <form className="terminal-find" role="search" aria-label="查找终端文本" onSubmit={(event) => {
          event.preventDefault();
          void find();
        }} onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            closeFind();
          } else if (event.key === "Enter" && event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            void find(true);
          }
        }}>
          <input ref={findInput} aria-label="查找终端文本" type="search" maxLength={256}
            value={query} placeholder="在终端中查找" onChange={(event) => {
              findRequest.current += 1;
              setQuery(event.target.value);
              setFound(undefined);
              finder.current?.clearDecorations();
            }} />
          {found === false && <span role="status">未找到</span>}
          <button type="button" disabled={!query} onClick={() => void find(true)}>上一个</button>
          <button type="submit" disabled={!query}>下一个</button>
          <button type="button" className="icon-button" aria-label="关闭查找" onClick={closeFind}>
            <Icon name="close" size={14} />
          </button>
        </form>
      )}
      {problem && (
        <div className="terminal-problem" role="alert">
          <span>终端通信异常：{problem}</span>
          <button
            onClick={() => {
              setProblem(undefined);
              setReady(false);
              setAttachment((n) => n + 1);
            }}
          >
            重新读取输出
          </button>
        </div>
      )}
      <div
        className="term"
        ref={box}
        data-writable={writable && !problem}
        aria-label={writable ? "远程终端，可输入" : "远程输出，只读"}
      />
    </div>
  );
}
