const VERSION = '@VERSION@';
const STATUS = ['ok', 'unsupported_type', 'too_large', 'malformed', 'verify_failed', 'binary', 'cancelled', 'invalid_argument', 'internal'];
const OK = 0;
const BINARY = 5;
const REASONS = [
  'none', 'bom', 'noncharacter', 'unassigned', 'private_use', 'bidi_control', 'join_control',
  'variation_selector', 'format', 'detached_mark', 'trailing_space', 'homoglyph', 'line_break',
  'invalid_utf8', 'control', 'mark_overflow',
];
const EDIT_REASONS = 12;
const RESULT_SIZE = 28;
const REPORT_SIZE = 136;
const SPANS = 24;
const BYTES = SPANS + 4 * EDIT_REASONS;
const AT = BYTES + 4 * EDIT_REASONS;

export class UnseenError extends Error {
  constructor(code, at) {
    super(code);
    this.name = 'UnseenError';
    this.code = code;
    if (at) this.at = at;
  }
}

export async function load(source = new URL('./unseen.wasm', import.meta.url)) {
  const module = await compile(source);
  const { exports } = await WebAssembly.instantiate(module, {});
  return new Unseen(wasm(exports));
}

async function compile(source) {
  if (source instanceof WebAssembly.Module) return source;
  if (typeof source === 'string') source = new URL(source, import.meta.url);
  if (source instanceof URL) {
    if (source.protocol === 'file:') {
      const { readFile } = await import('node:fs/promises');
      return WebAssembly.compile(await readFile(source));
    }
    source = fetch(source);
  }
  if (source instanceof Promise || source instanceof Response) {
    return WebAssembly.compileStreaming(source);
  }
  return WebAssembly.compile(source);
}

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });

// TextEncoder writes U+FFFD for a lone surrogate; its generalized UTF-8 bytes
// let the policy reject it instead.
function encode(s) {
  if (s.isWellFormed()) return encoder.encode(s);
  const bytes = new Uint8Array(s.length * 3);
  let n = 0;
  for (let i = 0; i < s.length; ) {
    const c = s.codePointAt(i);
    i += c > 0xffff ? 2 : 1;
    if (c < 0x80) {
      bytes[n++] = c;
    } else if (c < 0x800) {
      bytes[n++] = 0xc0 | (c >> 6);
      bytes[n++] = 0x80 | (c & 0x3f);
    } else if (c < 0x10000) {
      bytes[n++] = 0xe0 | (c >> 12);
      bytes[n++] = 0x80 | ((c >> 6) & 0x3f);
      bytes[n++] = 0x80 | (c & 0x3f);
    } else {
      bytes[n++] = 0xf0 | (c >> 18);
      bytes[n++] = 0x80 | ((c >> 12) & 0x3f);
      bytes[n++] = 0x80 | ((c >> 6) & 0x3f);
      bytes[n++] = 0x80 | (c & 0x3f);
    }
  }
  return bytes.subarray(0, n);
}

function unitCursor(s) {
  let unit = 0;
  let byte = 0;
  return (target) => {
    while (byte < target) {
      const c = s.codePointAt(unit);
      byte += c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
      unit += c > 0xffff ? 2 : 1;
    }
    return unit;
  };
}

class Unseen {
  #b;

  constructor(backend) {
    // Nothing else is read before this check: another build may lay out its
    // structures differently.
    if (backend.version() !== VERSION) throw new UnseenError('version');
    this.#b = backend;
    this.version = VERSION;
    this.unicodeVersion = backend.unicodeVersion();
  }

  sanitizeText(input) {
    const bytes = typeof input === 'string' ? encode(input) : input;
    const r = this.#b.sanitizeText(bytes);
    if (r.status !== OK) throw new UnseenError(STATUS[r.status] ?? 'internal');
    return typeof input === 'string' ? decoder.decode(r.output) : r.output;
  }

  sanitizeTextReport(input) {
    const text = typeof input === 'string';
    const bytes = text ? encode(input) : input;
    const r = this.#b.sanitizeTextReport(bytes);
    const unit = text ? unitCursor(input) : (b) => b;
    const place = (q, i) => {
      const f = { start: unit(q[i]), end: unit(q[i + 1]), reason: REASONS[q[i + 2]] ?? 'unknown' };
      if (q[i + 3] !== 0) f.with = String.fromCodePoint(q[i + 3]);
      return f;
    };
    if (r.status !== OK) {
      throw new UnseenError(STATUS[r.status] ?? 'internal', r.at[2] === 0 ? undefined : place(r.at, 0));
    }
    const edits = Array.from({ length: r.edits.length / 4 }, (_, i) => place(r.edits, 4 * i));
    const totals = {};
    for (let i = 0; i < EDIT_REASONS; i++) {
      totals[REASONS[i + 1]] = { spans: r.spans[i], bytes: r.bytes[i] };
    }
    return {
      output: text ? decoder.decode(r.output) : r.output,
      edits,
      truncated: r.truncated,
      totals,
    };
  }

  sanitizeFile(name, body) {
    const r = this.#b.sanitizeFile(encoder.encode(name), body);
    if (r.status === OK) return { kind: 'text', mime: r.mime, ext: r.ext, output: r.output };
    if (r.status === BINARY) return { kind: 'binary', mime: r.mime, ext: r.ext };
    throw new UnseenError(STATUS[r.status] ?? 'internal');
  }
}

function wasm(x) {
  const cString = (p) => {
    const memory = new Uint8Array(x.memory.buffer);
    let end = p;
    while (memory[end] !== 0) end++;
    return decoder.decode(memory.subarray(p, end));
  };

  const withBytes = (bytes, use) => {
    const n = bytes.length;
    const p = x.unseen_alloc(n);
    if (n > 0 && p === 0) throw new UnseenError('too_large');
    try {
      new Uint8Array(x.memory.buffer, p, n).set(bytes);
      return use(p, n);
    } finally {
      x.unseen_dealloc(p, n);
    }
  };

  const call = (size, invoke, free, read) => {
    const out = x.unseen_alloc(size);
    if (out === 0) throw new UnseenError('internal');
    try {
      invoke(out);
      // Read only after the call: memory may have grown and detached old views.
      const { buffer } = x.memory;
      const result = read(new DataView(buffer, out, size), buffer, out);
      x[free](out);
      return result;
    } finally {
      x.unseen_dealloc(out, size);
    }
  };

  const u32 = (buffer, p, n) => new Uint32Array(buffer.slice(p, p + 4 * n));

  const result = (invoke) =>
    call(RESULT_SIZE, invoke, 'unseen_result_free', (view, buffer) => {
      const field = (offset) => view.getUint32(offset, true);
      const text = (p, n) => decoder.decode(new Uint8Array(buffer, p, n));
      return {
        status: field(0),
        mime: text(field(4), field(8)),
        ext: text(field(12), field(16)),
        output: new Uint8Array(buffer, field(20), field(24)).slice(),
      };
    });

  const report = (invoke) =>
    call(REPORT_SIZE, invoke, 'unseen_report_free', (view, buffer, out) => {
      const field = (offset) => view.getUint32(offset, true);
      return {
        status: field(0),
        output: new Uint8Array(buffer, field(4), field(8)).slice(),
        edits: u32(buffer, field(12), 4 * field(16)),
        truncated: view.getUint8(20) !== 0,
        spans: u32(buffer, out + SPANS, EDIT_REASONS),
        bytes: u32(buffer, out + BYTES, EDIT_REASONS),
        at: u32(buffer, out + AT, 4),
      };
    });

  return {
    version: () => (typeof x.unseen_version === 'function' ? cString(x.unseen_version()) : ''),
    unicodeVersion: () => cString(x.unseen_unicode_version()),
    sanitizeText: (bytes) => result((out) => withBytes(bytes, (p, n) => x.unseen_sanitize_text(p, n, 0, 0, out))),
    sanitizeTextReport: (bytes) =>
      report((out) => withBytes(bytes, (p, n) => x.unseen_sanitize_text_report(p, n, 0, 0, out))),
    sanitizeFile: (name, body) =>
      result((out) =>
        withBytes(name, (np, nn) => withBytes(body, (bp, bn) => x.unseen_sanitize_file(np, nn, bp, bn, 0, 0, out))),
      ),
  };
}
