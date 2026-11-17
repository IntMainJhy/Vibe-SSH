/** Read-only polling: one request in flight, one queued refresh, no hidden-page timers. */
export function createReadPoller<T>({
  read,
  onValue,
  onError,
  onBusy,
  visible = () => true,
  interval = 5000,
}: {
  read: () => Promise<T>;
  onValue: (value: T) => void;
  onError: (error: unknown) => void;
  onBusy: (busy: boolean) => void;
  visible?: () => boolean;
  interval?: number;
}) {
  let active = true;
  let reading = false;
  let queued = false;
  let failures = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const cancelTimer = () => {
    clearTimeout(timer);
    timer = undefined;
  };
  const refresh = async (): Promise<void> => {
    cancelTimer();
    if (!active || !visible()) return;
    if (reading) {
      queued = true;
      return;
    }
    reading = true;
    onBusy(true);
    try {
      const value = await read();
      if (active) {
        failures = 0;
        onValue(value);
      }
    } catch (error) {
      if (active) {
        failures = Math.min(failures + 1, 6);
        onError(error);
      }
    } finally {
      reading = false;
      if (active) {
        onBusy(false);
        if (visible()) {
          if (queued) {
            queued = false;
            void refresh();
          } else {
            const delay = Math.min(interval * 2 ** Math.max(0, failures - 1), 30000);
            timer = setTimeout(() => void refresh(), delay);
          }
        }
      }
    }
  };
  return {
    refresh,
    visibilityChanged() {
      cancelTimer();
      queued = false;
      if (visible()) void refresh();
    },
    stop() {
      active = false;
      queued = false;
      cancelTimer();
    },
  };
}
