/** Split at Unicode boundaries; every transport write stays below the 32 KiB kernel limit. */
export function inputFrames(text: string, limit = 16_384): string[] {
  if (limit < 4)
    throw new RangeError("the frame limit must fit one Unicode code point");
  const encoder = new TextEncoder();
  const frames: string[] = [];
  let frame = "",
    size = 0;
  for (const character of text) {
    const bytes = encoder.encode(character).length;
    if (size + bytes > limit) {
      frames.push(frame);
      frame = "";
      size = 0;
    }
    frame += character;
    size += bytes;
  }
  if (frame) frames.push(frame);
  return frames;
}

/** Stops queued writes after a failure or loss of input control; never retries a write. */
export function inputQueue(
  allowed: () => boolean,
  failed: (error: unknown) => void,
  generation: () => unknown = () => 0,
) {
  let tail: Promise<void> = Promise.resolve();
  let halted = false;
  return (write: () => Promise<unknown>): Promise<void> => {
    const enqueued = generation();
    tail = tail.then(async () => {
      if (halted || !allowed() || enqueued !== generation()) return;
      try {
        await write();
      } catch (error) {
        halted = true;
        failed(error);
      }
    });
    return tail;
  };
}
