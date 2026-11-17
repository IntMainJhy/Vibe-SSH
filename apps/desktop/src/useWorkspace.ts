import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { call, explain, type Snapshot } from "./api";

/** One snapshot in flight; changes arriving during it trigger one more read. */
export function useWorkspace() {
  const [data, setData] = useState<Snapshot>();
  const [error, setError] = useState<string>();
  const [refreshing, setRefreshing] = useState(false);
  const active = useRef(false);
  const reading = useRef(false);
  const queued = useRef(false);
  const refresh = useCallback(async () => {
    if (reading.current) {
      queued.current = true;
      return;
    }
    reading.current = true;
    if (active.current) setRefreshing(true);
    try {
      do {
        queued.current = false;
        try {
          const next = await call<Snapshot>("snapshot");
          if (active.current) {
            setData(next);
            setError(undefined);
          }
        } catch (e) {
          if (active.current) setError(explain(e));
        }
      } while (active.current && queued.current);
    } finally {
      reading.current = false;
      if (active.current) setRefreshing(false);
    }
  }, []);
  useEffect(() => {
    active.current = true;
    const stop = listen("changed", () => void refresh());
    void stop.then(() => refresh()).catch((e) => setError(explain(e)));
    // Detect a quiet connection that has dropped without a running job to publish a change.
    const poll = setInterval(() => void refresh(), 5000);
    return () => {
      active.current = false;
      clearInterval(poll);
      void stop.then((off) => off()).catch(() => {});
    };
  }, [refresh]);
  return { data, error, refreshing, refresh };
}
