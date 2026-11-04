# unseen.rs

Очищает текст от невидимых символов, которыми его можно пометить или подменить: удаляет форматирующие и bidi-символы, теги, соединители вне emoji, приватную зону, заменяет буквы-двойники другого алфавита, снимает хвостовые пробелы и приводит переводы строк к LF. Отклоняет управляющие символы и невалидный UTF-8. Для файла определяет тип по имени и содержимому и очищает текстовые форматы.

## Установка

Rust:

```bash
cargo add unseen-core
```

JavaScript:

```bash
pnpm add @meritt/unseen-wasm
```

Swift:

```swift
.package(url: "https://github.com/meritt/unseen.rs", exact: "0.1.0")
```

## Использование

Rust:

```rust
let clean = unseen_core::sanitize_text("a\u{200B}b".as_bytes())?; // b"ab"
```

JavaScript:

```js
import { load } from '@meritt/unseen-wasm';

const unseen = await load();
unseen.sanitizeText('a\u{200B}b');                  // 'ab'
unseen.sanitizeFile('notes.md', bytes);             // { kind: 'text', mime, ext, output }
unseen.sanitizeTextReport('a\u{200B}b \u{430}pple');
// { output: 'ab apple', truncated: false, totals, edits: [
//   { start: 1, end: 2, reason: 'format' },
//   { start: 4, end: 5, reason: 'homoglyph', with: 'a' } ] }
```

Swift:

```swift
import Unseen

let clean = try Unseen.sanitize("a\u{200B}b")       // "ab"
let report = try Unseen.sanitizeReport(text)        // report.edits: ranges over text.unicodeScalars
```

Отказ — ошибка с кодом: `unsupported_type`, `too_large`, `malformed`, `verify_failed`, `cancelled`.

## Автор

[Алексей Симоненко](https://simonenko.xyz)

## Лицензия

MIT. Смотрите файл `LICENSE`.
