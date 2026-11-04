import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

const consumer = process.env.UNSEEN_PACKAGE;

async function entry() {
  if (!consumer) return new URL('../dist/unseen.js', import.meta.url).href;
  const manifest = resolve(consumer, 'package.json');
  const deps = Object.keys(JSON.parse(await readFile(manifest, 'utf8')).dependencies);
  const name = deps.find((d) => /^(@[^/]+\/)?unseen-wasm$/.test(d));
  return pathToFileURL(createRequire(manifest).resolve(name)).href;
}

const { UnseenError, load } = await import(await entry());

const unseen = await load();

const corpusDir =
  process.env.UNSEEN_CORPUS ?? fileURLToPath(new URL('../../../testdata/textcorpus/', import.meta.url));

async function corpus(file) {
  const value = JSON.parse(await readFile(`${corpusDir}/${file}`, 'utf8'));
  assert.equal(value.format, 1);
  return value;
}

const encoder = new TextEncoder();
const unhex = (s) => Uint8Array.from(Buffer.from(s, 'hex'));
const hex = (b) => Buffer.from(b).toString('hex');
const sha256 = (b) => createHash('sha256').update(b).digest('hex');

function expand(input) {
  return Buffer.concat(input.flatMap(({ hex: h, repeat }) => Array(repeat).fill(Buffer.from(h, 'hex'))));
}

function checkOutput(name, want, got) {
  assert.equal(got.length, want.output.len, `${name}: output length`);
  assert.equal(sha256(got), want.output.sha256, `${name}: output digest`);
  if (want.output.hex !== undefined) assert.equal(hex(got), want.output.hex, `${name}: output bytes`);
}

function splitmix64(seed) {
  let state = BigInt(seed);
  const mask = (1n << 64n) - 1n;
  const next = () => {
    state = (state + 0x9e3779b97f4a7c15n) & mask;
    let z = state;
    z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & mask;
    z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & mask;
    return z ^ (z >> 31n);
  };
  return { below: (n) => Number(next() % BigInt(n)) };
}

const FILE_CODES = ['ok', 'unsupported_type', 'too_large', 'malformed', 'verify_failed', 'binary'];

function fileResult(name, body) {
  try {
    const r = unseen.sanitizeFile(name, body);
    return r.kind === 'text'
      ? { result: 'ok', mime: r.mime, ext: r.ext, output: r.output }
      : { result: 'binary', mime: r.mime, ext: r.ext };
  } catch (err) {
    if (!(err instanceof UnseenError)) throw err;
    return { result: err.code, mime: '', ext: '' };
  }
}

test('file corpus cases', async () => {
  const c = await corpus('files.json');
  for (const { name, file_name: fileName, input, want } of c.cases) {
    const got = fileResult(fileName, expand(input));
    assert.equal(got.result, want.result, `${name}: result`);
    assert.equal(got.mime, want.mime ?? '', `${name}: mime`);
    assert.equal(got.ext, want.ext ?? '', `${name}: ext`);
    if (got.output) checkOutput(name, want, got.output);
    else assert.equal(want.output, undefined, `${name}: output recorded for a refusal`);
  }
});

test('file corpus random blocks', async () => {
  const c = await corpus('files.json');
  const { random } = c;
  const alphabet = random.alphabet.map(unhex);
  const rng = splitmix64(random.seed);
  assert.equal(random.blocks_sha256.length * random.block_size, random.cases);
  let h = createHash('sha256');
  for (let i = 0; i < random.cases; i++) {
    const name = random.file_names[rng.below(random.file_names.length)];
    const n = 1 + rng.below(random.max_tokens);
    const body = Buffer.concat(Array.from({ length: n }, () => alphabet[rng.below(alphabet.length)]));
    const r = fileResult(name, body);
    const mime = Buffer.from(r.mime);
    const ext = Buffer.from(r.ext);
    h.update(Uint8Array.of(FILE_CODES.indexOf(r.result), mime.length));
    h.update(mime);
    h.update(Uint8Array.of(ext.length));
    h.update(ext);
    if (r.output) {
      const len = Buffer.alloc(4);
      len.writeUInt32BE(r.output.length);
      h.update(len);
      h.update(r.output);
    }
    if ((i + 1) % random.block_size === 0) {
      const block = i / random.block_size | 0;
      assert.equal(h.digest('hex'), random.blocks_sha256[block], `random block ${block}`);
      h = createHash('sha256');
    }
  }
});

function refusal(f) {
  try {
    return { value: f() };
  } catch (err) {
    if (!(err instanceof UnseenError)) throw err;
    return { err };
  }
}

function checkReport(name, input, want) {
  const { value: report, err } = refusal(() => unseen.sanitizeTextReport(input));
  if (want.result !== 'ok') {
    assert.equal(err?.code, want.result, `${name}: report result`);
    if (err.at) assert.ok(err.at.start < err.at.end && err.at.end <= input.length, `${name}: refusal bounds`);
    return;
  }
  assert.ok(report, `${name}: report refused with ${err?.code}`);
  checkOutput(name, want, report.output);
  const parts = [];
  const listed = {};
  let cursor = 0;
  for (const { start, end, reason, with: w } of report.edits) {
    assert.ok(cursor <= start && start < end && end <= input.length, `${name}: fragment order`);
    parts.push(input.subarray(cursor, start));
    if (w !== undefined) parts.push(encoder.encode(w));
    cursor = end;
    listed[reason] ??= { spans: 0, bytes: 0 };
    listed[reason].spans += 1;
    listed[reason].bytes += end - start;
  }
  parts.push(input.subarray(cursor));
  const rebuilt = Buffer.concat(parts);
  assert.equal(!rebuilt.equals(report.output), report.truncated, `${name}: input with edits applied`);
  if (!report.truncated) {
    for (const [reason, total] of Object.entries(report.totals)) {
      assert.deepEqual(total, listed[reason] ?? { spans: 0, bytes: 0 }, `${name}: ${reason} total`);
    }
  }
}

test('text corpus cases', async () => {
  const c = await corpus('text.json');
  assert.equal(unseen.unicodeVersion, c.unicode_version);
  for (const { name, slow, input, want } of c.cases) {
    if (slow) continue;
    const bytes = expand(input);
    checkReport(name, bytes, want);
    let output;
    try {
      output = unseen.sanitizeText(bytes);
    } catch (err) {
      if (!(err instanceof UnseenError)) throw err;
      assert.equal(err.code, want.result, `${name}: result`);
      continue;
    }
    assert.equal('ok', want.result, `${name}: result`);
    checkOutput(name, want, output);
  }
});

test('report offsets', () => {
  const input = '\u{1F600}a\u{200B}b\u{202E}\u{1F469}\u{200D}\u{1F4BB}';
  const r = unseen.sanitizeTextReport(input);
  assert.equal(r.output, '\u{1F600}ab\u{1F469}\u{200D}\u{1F4BB}');
  assert.deepEqual(r.edits, [
    { start: 3, end: 4, reason: 'format' },
    { start: 5, end: 6, reason: 'bidi_control' },
  ]);
  assert.equal(input.slice(3, 4), '\u{200B}');
  assert.deepEqual(r.totals.format, { spans: 1, bytes: 3 });
  assert.equal(r.truncated, false);

  const bytes = unseen.sanitizeTextReport(encoder.encode(input));
  assert.deepEqual(bytes.edits, [
    { start: 5, end: 8, reason: 'format' },
    { start: 9, end: 12, reason: 'bidi_control' },
  ]);

  const { err } = refusal(() => unseen.sanitizeTextReport('\u{1F600}\u{200B}x\0\0y'));
  assert.equal(err.code, 'malformed');
  assert.deepEqual(err.at, { start: 4, end: 6, reason: 'control' });
  assert.equal(refusal(() => unseen.sanitizeText('a\0')).err.at, undefined);
});

test('lone surrogates', () => {
  assert.throws(() => unseen.sanitizeText('a\u{D800}b'), { name: 'UnseenError', code: 'malformed' });
  assert.throws(() => unseen.sanitizeText('a\u{DC00}'), { name: 'UnseenError', code: 'malformed' });
  const at = (s) => refusal(() => unseen.sanitizeTextReport(s)).err?.at;
  assert.deepEqual(at('\u{1F600}\u{200B}x\u{DC00}y'), { start: 4, end: 5, reason: 'invalid_utf8' });
  assert.deepEqual(at('\u{D83D}\u{D83D}\u{DE00}'), { start: 0, end: 1, reason: 'invalid_utf8' });
  assert.deepEqual(at('a\0\u{D800}b'), { start: 1, end: 3, reason: 'control' });
});

test('report edits', () => {
  const input = '\u{1F600} \u{430}pple.com \r\nx\u{2028}';
  const r = unseen.sanitizeTextReport(input);
  assert.equal(r.output, '\u{1F600} apple.com\nx\n');
  assert.deepEqual(r.edits, [
    { start: 3, end: 4, reason: 'homoglyph', with: 'a' },
    { start: 12, end: 13, reason: 'trailing_space' },
    { start: 13, end: 14, reason: 'line_break' },
    { start: 16, end: 17, reason: 'line_break', with: '\n' },
  ]);
  assert.deepEqual(r.totals.homoglyph, { spans: 1, bytes: 2 });
  assert.deepEqual(r.totals.line_break, { spans: 2, bytes: 4 });
});

test('report cap', () => {
  const n = 100_005;
  const r = unseen.sanitizeTextReport('a\u{200B}'.repeat(n));
  assert.equal(r.output, 'a'.repeat(n));
  assert.equal(r.edits.length, 100_000);
  assert.equal(r.truncated, true);
  assert.deepEqual(r.totals.format, { spans: n, bytes: 3 * n });
  assert.deepEqual(r.edits.at(-1), { start: 199_999, end: 200_000, reason: 'format' });
});

test('module version', async () => {
  const manifest = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));
  assert.equal(unseen.version, manifest.version);
  const empty = new WebAssembly.Module(Uint8Array.of(0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00));
  await assert.rejects(load(empty), { name: 'UnseenError', code: 'version' });
});

test('strings round-trip', () => {
  assert.equal(unseen.sanitizeText('Съешь​ещё \u{1F469}‍\u{1F4BB}'), 'Съешьещё \u{1F469}‍\u{1F4BB}');
  assert.throws(() => unseen.sanitizeText('a\0b'), { name: 'UnseenError', code: 'malformed' });
});
