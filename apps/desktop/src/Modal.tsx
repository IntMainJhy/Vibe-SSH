import { useEffect, useRef, type ReactNode, type RefObject } from "react";

/** Native modal semantics provide keyboard trapping, Escape and focus restoration. */
export function Modal({
  titleId,
  onClose,
  busy = false,
  children,
  className = "",
  initialFocus,
}: {
  titleId: string;
  onClose: () => void;
  busy?: boolean;
  children: ReactNode;
  className?: string;
  initialFocus?: RefObject<HTMLElement | null>;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const node = dialog.current!;
    const previous = document.activeElement as HTMLElement | null;
    node.showModal();
    initialFocus?.current?.focus();
    return () => {
      node.close();
      previous?.focus();
    };
  }, [initialFocus]);
  return (
    <dialog
      ref={dialog}
      className={`dialog ${className}`}
      aria-labelledby={titleId}
      onKeyDown={(event) => {
        // Handle Escape before the browser close watcher: repeated close requests
        // can produce a non-cancelable native cancel event and hide a dirty editor.
        if (event.key !== "Escape") return;
        event.preventDefault();
        event.stopPropagation();
        if (!busy) close.current();
      }}
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) close.current();
      }}
    >
      {children}
    </dialog>
  );
}
