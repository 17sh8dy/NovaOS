import { useEffect, useState } from 'react';
import { filesApi } from '../apps/files/filesApi';
import { isTauri } from '../native/bridge';
import { CURSOR_DIR, isCursorFileName, WALLPAPER_ROOT, type CursorHotspot } from './schema';

/**
 * Reading a cursor file the user added. Windows `.cur` and animated `.ani` files, PNG and GIF are
 * decoded here as plain image data; nothing in them is ever executed. Every length and offset is
 * bounds-checked, so a corrupt or hostile file just fails to load and NovaOS keeps its normal
 * pointer.
 */

export interface CursorFrame {
  /** An image file the browser can draw: a .cur/.png/.gif, as bytes. */
  data: Uint8Array;
  mime: string;
  width: number;
  height: number;
  /** Where the pointer "points", in image pixels. Null when the file doesn't say (PNG, GIF). */
  hotspot: [number, number] | null;
}

export interface DecodedCursor {
  frames: CursorFrame[];
  /** The order frames are shown in (indices into `frames`). One entry for a still cursor. */
  sequence: number[];
  /** How long each step of the sequence shows, in milliseconds. */
  delays: number[];
}

export const MAX_CURSOR_PX = 128;
const MAX_FRAMES = 128;
const MAX_STEPS = 512;

const u16 = (b: Uint8Array, o: number): number => b[o] | (b[o + 1] << 8);
const u32 = (b: Uint8Array, o: number): number =>
  (b[o] | (b[o + 1] << 8) | (b[o + 2] << 16) | (b[o + 3] << 24)) >>> 0;
const tag = (b: Uint8Array, o: number): string =>
  String.fromCharCode(b[o], b[o + 1], b[o + 2], b[o + 3]);

/** A single Windows .cur file: the largest image in it, with its hotspot. */
export function parseCur(bytes: Uint8Array): CursorFrame | null {
  if (bytes.length < 22 || u16(bytes, 0) !== 0 || u16(bytes, 2) !== 2) return null;
  const count = u16(bytes, 4);
  if (count < 1 || count > 64 || bytes.length < 6 + count * 16) return null;
  let best: CursorFrame | null = null;
  for (let i = 0; i < count; i++) {
    const o = 6 + i * 16;
    const width = bytes[o] || 256;
    const height = bytes[o + 1] || 256;
    const size = u32(bytes, o + 8);
    const offset = u32(bytes, o + 12);
    if (size === 0 || offset + size > bytes.length) return null;
    if (!best || width * height > best.width * best.height) {
      best = {
        data: bytes,
        mime: 'image/x-icon',
        width,
        height,
        hotspot: [Math.min(u16(bytes, o + 4), width - 1), Math.min(u16(bytes, o + 6), height - 1)],
      };
    }
  }
  return best;
}

function parseStill(ext: string, bytes: Uint8Array): CursorFrame | null {
  if (ext === 'png') {
    if (bytes.length < 24 || tag(bytes, 12) !== 'IHDR') return null;
    const width = (bytes[16] << 24) | (bytes[17] << 16) | (bytes[18] << 8) | bytes[19];
    const height = (bytes[20] << 24) | (bytes[21] << 16) | (bytes[22] << 8) | bytes[23];
    if (width < 1 || height < 1 || width > 4096 || height > 4096) return null;
    return { data: bytes, mime: 'image/png', width, height, hotspot: null };
  }
  if (ext === 'gif') {
    if (bytes.length < 10) return null;
    const width = u16(bytes, 6);
    const height = u16(bytes, 8);
    if (width < 1 || height < 1) return null;
    return { data: bytes, mime: 'image/gif', width, height, hotspot: null };
  }
  return null;
}

/** A Windows animated cursor (.ani): a RIFF file holding one .cur per frame plus timing. */
export function parseAni(bytes: Uint8Array): DecodedCursor | null {
  if (bytes.length < 12 || tag(bytes, 0) !== 'RIFF' || tag(bytes, 8) !== 'ACON') return null;
  let jiffies = 6;
  let flags = 1;
  let steps = 0;
  let rates: number[] = [];
  let seq: number[] = [];
  const frames: CursorFrame[] = [];

  const walk = (start: number, end: number, inFrames: boolean): boolean => {
    let o = start;
    while (o + 8 <= end) {
      const id = tag(bytes, o);
      const size = u32(bytes, o + 4);
      const body = o + 8;
      if (body + size > end) return false;
      if (id === 'anih' && size >= 36) {
        steps = u32(bytes, body + 8);
        jiffies = u32(bytes, body + 28);
        flags = u32(bytes, body + 32);
      } else if (id === 'rate') {
        rates = [];
        for (let i = 0; i < Math.min(size / 4, MAX_STEPS); i++)
          rates.push(u32(bytes, body + i * 4));
      } else if (id === 'seq ') {
        seq = [];
        for (let i = 0; i < Math.min(size / 4, MAX_STEPS); i++) seq.push(u32(bytes, body + i * 4));
      } else if (id === 'LIST' && size >= 4) {
        const kind = tag(bytes, body);
        if (kind === 'fram' && !walk(body + 4, body + size, true)) return false;
      } else if (id === 'icon' && inFrames) {
        if (frames.length >= MAX_FRAMES) return false;
        const f = parseCur(bytes.subarray(body, body + size));
        if (!f) return false;
        frames.push(f);
      }
      o = body + size + (size & 1); // chunks are padded to an even length
    }
    return true;
  };

  if (!walk(12, bytes.length, false)) return null;
  if ((flags & 1) === 0 || frames.length === 0) return null; // raw-bitmap frames aren't supported

  const sequence = seq.length > 0 ? seq : frames.map((_, i) => i);
  if (sequence.length > MAX_STEPS || sequence.some((i) => i >= frames.length)) return null;
  const count = steps > 0 ? Math.min(steps, sequence.length) : sequence.length;
  const stepJiffies = (i: number) => (rates.length > i ? rates[i] : jiffies) || 6;
  const used = sequence.slice(0, count);
  return {
    frames,
    sequence: used,
    // A jiffy is 1/60 s. Never faster than ~30 fps, never longer than 2 s.
    delays: used.map((_, i) =>
      Math.min(2000, Math.max(33, Math.round((stepJiffies(i) * 1000) / 60))),
    ),
  };
}

/** Decode a cursor file by its extension. Null if it isn't a usable cursor. */
export function decodeCursorFile(name: string, bytes: Uint8Array): DecodedCursor | null {
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  if (ext === 'ani') return parseAni(bytes);
  const frame = ext === 'cur' ? parseCur(bytes) : parseStill(ext, bytes);
  // Big cursors are scaled down when drawn; only the absurd are refused.
  if (!frame || frame.width > 1024 || frame.height > 1024) return null;
  return { frames: [frame], sequence: [0], delays: [1000] };
}

/** How to draw a frame: displayed size and hotspot, scaled by the Pointer size setting. */
export function framePose(
  frame: Pick<CursorFrame, 'width' | 'height' | 'hotspot'>,
  size: number,
  mode: CursorHotspot,
): { width: number; height: number; hotspot: [number, number] } {
  // 32 is the classic cursor size, so the slider's default shows a cursor at its natural size.
  let k = size / 32;
  const biggest = Math.max(frame.width, frame.height) * k;
  if (biggest > MAX_CURSOR_PX) k *= MAX_CURSOR_PX / biggest;
  const [hx, hy] =
    frame.hotspot ?? (mode === 'center' ? [frame.width / 2, frame.height / 2] : [0, 0]);
  return {
    width: Math.max(1, Math.round(frame.width * k)),
    height: Math.max(1, Math.round(frame.height * k)),
    hotspot: [Math.round(hx * k), Math.round(hy * k)],
  };
}

// ---------------------------------------------------------------- loading in the app

export interface LoadedFrame {
  url: string;
  width: number;
  height: number;
  hotspot: [number, number] | null;
}

export interface LoadedCursor {
  frames: LoadedFrame[];
  sequence: number[];
  delays: number[];
}

export type CursorStatus = 'idle' | 'loading' | 'ready' | 'failed';

/**
 * Loads a cursor file (from the Cursors folder, through the sandboxed filesystem) into image
 * URLs. Anything that doesn't decode comes back `failed`, and NovaOS keeps its normal pointer.
 */
export function useCustomCursor(file: string | null): {
  cursor: LoadedCursor | null;
  status: CursorStatus;
} {
  const [state, setState] = useState<{
    file: string | null;
    cursor: LoadedCursor | null;
    status: CursorStatus;
  }>({ file: null, cursor: null, status: 'idle' });

  useEffect(() => {
    if (!file || !isCursorFileName(file)) return;
    if (!isTauri()) {
      setState({ file, cursor: null, status: 'failed' });
      return;
    }
    let alive = true;
    const urls: string[] = [];
    setState({ file, cursor: null, status: 'loading' });
    filesApi
      .readBinary(WALLPAPER_ROOT, `${CURSOR_DIR}/${file}`)
      .then((buf) => {
        if (!alive) return;
        const decoded = decodeCursorFile(file, new Uint8Array(buf));
        if (!decoded) throw new Error('not a usable cursor');
        const frames: LoadedFrame[] = decoded.frames.map((f) => {
          const url = URL.createObjectURL(new Blob([f.data as BlobPart], { type: f.mime }));
          urls.push(url);
          return { url, width: f.width, height: f.height, hotspot: f.hotspot };
        });
        const cursor = { frames, sequence: decoded.sequence, delays: decoded.delays };
        // Make sure the browser can really draw it before NovaOS hides the normal pointer for it.
        const probe = new Image();
        probe.onload = () => alive && setState({ file, cursor, status: 'ready' });
        probe.onerror = () => alive && setState({ file, cursor: null, status: 'failed' });
        probe.src = frames[0].url;
      })
      .catch(() => alive && setState({ file, cursor: null, status: 'failed' }));
    return () => {
      alive = false;
      urls.forEach((u) => URL.revokeObjectURL(u));
    };
  }, [file]);

  if (!file) return { cursor: null, status: 'idle' };
  return state.file === file
    ? { cursor: state.cursor, status: state.status }
    : { cursor: null, status: 'loading' };
}
