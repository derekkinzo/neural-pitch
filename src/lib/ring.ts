// Typed circular buffer for hot-path PitchUpdate frames.
//
// Held in a `useRef` by `usePitchStream`, never in Zustand: the per-frame
// rAF loop in `CentsMeter` and `HistoryStrip` reads this directly so React
// never re-renders on every analysis frame (~93 Hz).
//
// `push` overwrites the oldest entry once `capacity` is reached. `peekLatest`
// returns the most recently pushed element in O(1) with no allocation. Callers
// MUST treat the returned reference as read-only — mutating it would corrupt
// the next push.
//
// Cross-references:
//   docs/design/DESIGN.md §7 (hot-path / no useState on per-frame path)
//   docs/adr/0003-stack-tauri-react-tailwind.md

export class RingBuffer<T> {
  private readonly buf: Array<T | undefined>;
  private head = 0;
  private size = 0;

  public constructor(public readonly capacity: number) {
    if (capacity <= 0 || !Number.isInteger(capacity)) {
      throw new RangeError(`RingBuffer capacity must be a positive integer; got ${capacity}`);
    }
    this.buf = new Array<T | undefined>(capacity).fill(undefined);
  }

  public push(value: T): void {
    this.buf[this.head] = value;
    this.head = (this.head + 1) % this.capacity;
    if (this.size < this.capacity) this.size += 1;
  }

  public get length(): number {
    return this.size;
  }

  public clear(): void {
    for (let i = 0; i < this.capacity; i += 1) this.buf[i] = undefined;
    this.head = 0;
    this.size = 0;
  }

  /** Most recently pushed element, or `undefined` when the ring is empty.
   *  O(1), no allocation. */
  public peekLatest(): T | undefined {
    if (this.size === 0) return undefined;
    const idx = (this.head - 1 + this.capacity) % this.capacity;
    return this.buf[idx];
  }

  /**
   * Visit the last `n` entries from oldest to newest. Bounded by `length`.
   * The callback receives the index from 0 (oldest of the requested window)
   * to `count - 1` (newest). Skipped if the ring is empty.
   *
   * Designed for HistoryStrip: it iterates without allocating an array.
   */
  public forEachLast(n: number, fn: (value: T, i: number) => void): void {
    const count = Math.min(n, this.size);
    if (count === 0) return;
    // Oldest of the requested window:
    const startBack = count;
    const start = (this.head - startBack + this.capacity) % this.capacity;
    for (let i = 0; i < count; i += 1) {
      const v = this.buf[(start + i) % this.capacity];
      if (v !== undefined) fn(v, i);
    }
  }
}
