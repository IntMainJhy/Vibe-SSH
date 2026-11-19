import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { call, explain, type Snapshot } from "./api";
import { createReadPoller } from "./read-poller";

/** One snapshot in flight; changes arriving during it trigger one more read. */
export function useWorkspace() {
  const [data, setData] = useState<Snapshot>();
  const [error, setError] = useState<string>();
  const poller = useRef<ReturnType<typeof createReadPoller<Snapshot>> | null>(null);
  const refresh = useCallback(async () => {
    await poller.current?.refresh();
  }, []);
  useEffect(() => {
    let active = true;
    const loop = createReadPoller({
      read: () => call<Snapshot>("snapshot"),
      onValue: (next) => {
        setData(next);
        setError(undefined);
      },
      onError: (e) => setError(explain(e)),
      visible: () => !document.hidden,
    });
    poller.current = loop;
    const stop = listen("changed", () => void loop.refresh());
    void stop.then(() => loop.refresh()).catch((e) => {
      if (active) setError(explain(e));
    });
    document.addEventListener("visibilitychange", loop.visibilityChanged);
    return () => {
      active = false;
      loop.stop();
      poller.current = null;
      document.removeEventListener("visibilitychange", loop.visibilityChanged);
      void stop.then((off) => off()).catch(() => {});
    };
  }, [refresh]);
  return { data, error, refresh };
}
