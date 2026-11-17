import { FitAddon } from "@xterm/addon-fit";
import { Terminal as Xterm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef } from "react";
import { call, report, type Chunk } from "./api";

const ENDED = { succeeded: "已结束", failed: "已失败", unknown: "结果未知，请到服务器上核实" };

type Props = { space: string; job: string; pty: boolean; writable: boolean };

/** Shows one job's output and, while the user holds the space, sends what they type. */
export function Terminal({ space, job, pty, writable }: Props) {
  const box = useRef<HTMLDivElement>(null);
  const term = useRef<Xterm | null>(null);
  const canWrite = useRef(writable);

  useEffect(() => {
    const xterm = new Xterm({
      convertEol: !pty,
      fontFamily: '"SF Mono", ui-monospace, Menlo, monospace',
      fontSize: 13,
      scrollback: 5000,
      theme: { background: "#131822", foreground: "#d8dee9", cursor: "#86a6ff", selectionBackground: "#3a4a6b" },
    });
    const fit = new FitAddon();
    xterm.loadAddon(fit);
    xterm.open(box.current!);
    term.current = xterm;

    let alive = true;
    // Replayed output can contain terminal queries; answering them again would type into the shell.
    let live = false;
    // One in flight at a time, so keystrokes reach the server in the order they were typed.
    let queue: Promise<unknown> = Promise.resolve();
    const send = (command: string, args: Record<string, unknown>) => {
      queue = queue.then(() => call(command, { space, job, ...args })).catch(() => {});
    };
    const typed = xterm.onData((data) => live && canWrite.current && send("job_input", { data }));
    const resized = xterm.onResize(
      ({ cols, rows }) => pty && canWrite.current && send("job_resize", { columns: cols, rows }),
    );
    // Size the terminal before anything is sent, so the first resize carries the real size.
    const refit = () => box.current?.clientWidth && fit.fit();
    refit();
    const observer = new ResizeObserver(refit);
    observer.observe(box.current!);

    (async () => {
      let next: [number, number] = [0, 0];
      while (alive) {
        let chunk: Chunk;
        try {
          // Replay what already exists without waiting; only then wait for new output.
          const waitMs = live ? 25_000 : 0;
          chunk = await call<Chunk>("job_stream", { job, out: next[0], err: next[1], waitMs });
        } catch {
          return; // the space was closed
        }
        if (!alive) return;
        if (chunk.gap) xterm.write("\r\n\x1b[2m[更早的输出已被清理]\x1b[0m\r\n");
        if (chunk.err.length) {
          xterm.write("\x1b[31m");
          xterm.write(Uint8Array.from(chunk.err));
          xterm.write("\x1b[0m");
        }
        if (chunk.out.length) {
          await new Promise<void>((done) => xterm.write(Uint8Array.from(chunk.out), done));
        }
        next = chunk.next;
        if (!live && chunk.out.length < 65_536) {
          live = true;
          report(`terminal ${xterm.cols}x${xterm.rows} in ${box.current?.clientWidth}px`);
        }
        if (chunk.status !== "running" && chunk.out.length + chunk.err.length === 0 && live) {
          const code = chunk.exit_code === null ? "" : `，退出码 ${chunk.exit_code}`;
          xterm.write(`\r\n\x1b[2m[${ENDED[chunk.status]}${code}]\x1b[0m\r\n`);
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
    };
  }, [space, job, pty]);

  useEffect(() => {
    canWrite.current = writable;
    const xterm = term.current;
    if (!xterm) return;
    xterm.options.disableStdin = !writable;
    xterm.options.cursorBlink = writable;
    if (!writable) return;
    xterm.focus();
    // The job may have been started at another size, by the agent.
    if (pty) call("job_resize", { space, job, columns: xterm.cols, rows: xterm.rows }).catch(() => {});
  }, [writable, space, job, pty]);

  return <div className="term" ref={box} data-writable={writable} />;
}
