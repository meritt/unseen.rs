#include "unseen.h"

#include <stdio.h>
#include <string.h>

static int failures;

static void expect(int ok, const char *what) {
    if (!ok) {
        fprintf(stderr, "header.c: %s\n", what);
        failures++;
    }
}

static int named(const char *got, const char *want) { return strcmp(got, want) == 0; }

static int bytes(const uint8_t *p, size_t len, const char *want) {
    return len == strlen(want) && (len == 0 || memcmp(p, want, len) == 0);
}

static int fragment(unseen_fragment f, size_t start, size_t end, uint32_t reason, uint32_t with) {
    return f.start == start && f.end == end && f.reason == reason && f.with == with;
}

static void statuses(void) {
    static const struct {
        uint32_t code;
        const char *name;
    } all[] = {
        {UNSEEN_OK, "ok"},
        {UNSEEN_UNSUPPORTED_TYPE, "unsupported_type"},
        {UNSEEN_TOO_LARGE, "too_large"},
        {UNSEEN_MALFORMED, "malformed"},
        {UNSEEN_VERIFY_FAILED, "verify_failed"},
        {UNSEEN_BINARY, "binary"},
        {UNSEEN_CANCELLED, "cancelled"},
        {UNSEEN_INVALID_ARGUMENT, "invalid_argument"},
        {UNSEEN_INTERNAL, "internal"},
    };
    for (size_t i = 0; i < sizeof all / sizeof all[0]; i++) {
        expect(named(unseen_status_name(all[i].code), all[i].name), all[i].name);
    }
}

static void reasons(void) {
    static const struct {
        uint32_t code;
        const char *name;
    } all[] = {
        {UNSEEN_REASON_NONE, "none"},
        {UNSEEN_REASON_BOM, "bom"},
        {UNSEEN_REASON_NONCHARACTER, "noncharacter"},
        {UNSEEN_REASON_UNASSIGNED, "unassigned"},
        {UNSEEN_REASON_PRIVATE_USE, "private_use"},
        {UNSEEN_REASON_BIDI_CONTROL, "bidi_control"},
        {UNSEEN_REASON_JOIN_CONTROL, "join_control"},
        {UNSEEN_REASON_VARIATION_SELECTOR, "variation_selector"},
        {UNSEEN_REASON_FORMAT, "format"},
        {UNSEEN_REASON_DETACHED_MARK, "detached_mark"},
        {UNSEEN_REASON_TRAILING_SPACE, "trailing_space"},
        {UNSEEN_REASON_HOMOGLYPH, "homoglyph"},
        {UNSEEN_REASON_LINE_BREAK, "line_break"},
        {UNSEEN_REASON_INVALID_UTF8, "invalid_utf8"},
        {UNSEEN_REASON_CONTROL, "control"},
        {UNSEEN_REASON_MARK_OVERFLOW, "mark_overflow"},
    };
    for (size_t i = 0; i < sizeof all / sizeof all[0]; i++) {
        expect(all[i].code == i, "reason codes are dense");
        expect(named(unseen_reason_name(all[i].code), all[i].name), all[i].name);
    }
    expect(UNSEEN_EDIT_REASONS == UNSEEN_REASON_LINE_BREAK, "UNSEEN_EDIT_REASONS is the last edit reason");
}

static void text(void) {
    static const char input[] = "a\xE2\x80\x8B" "b";
    unseen_result r;
    memset(&r, 0xA5, sizeof r);
    uint32_t status = unseen_sanitize_text((const uint8_t *)input, strlen(input), NULL, NULL, &r);
    expect(status == UNSEEN_OK && r.status == UNSEEN_OK, "sanitize_text status");
    expect(bytes(r.output, r.output_len, "ab"), "sanitize_text output");
    unseen_result_free(&r);
    expect(r.output == NULL && r.output_len == 0, "result_free clears the output");
    unseen_result_free(&r);
    unseen_result_free(NULL);
}

static void report(void) {
    static const char input[] = "\xD0\xB0pple \r\nx\xE2\x80\xA8";
    unseen_report r;
    memset(&r, 0xA5, sizeof r);
    uint32_t status = unseen_sanitize_text_report((const uint8_t *)input, strlen(input), NULL, NULL, &r);
    expect(status == UNSEEN_OK && r.status == UNSEEN_OK, "report status");
    expect(bytes(r.output, r.output_len, "apple\nx\n"), "report output");
    expect(r.edits_len == 4 && !r.truncated, "report edits");
    if (r.edits_len == 4) {
        expect(fragment(r.edits[0], 0, 2, UNSEEN_REASON_HOMOGLYPH, 'a'), "homoglyph edit");
        expect(fragment(r.edits[1], 6, 7, UNSEEN_REASON_TRAILING_SPACE, 0), "trailing space edit");
        expect(fragment(r.edits[2], 7, 8, UNSEEN_REASON_LINE_BREAK, 0), "crlf edit");
        expect(fragment(r.edits[3], 10, 13, UNSEEN_REASON_LINE_BREAK, '\n'), "line separator edit");
    }
    for (uint32_t reason = 1; reason <= UNSEEN_EDIT_REASONS; reason++) {
        size_t spans = reason == UNSEEN_REASON_LINE_BREAK ? 2 : reason == UNSEEN_REASON_HOMOGLYPH || reason == UNSEEN_REASON_TRAILING_SPACE ? 1 : 0;
        size_t total = reason == UNSEEN_REASON_LINE_BREAK ? 4 : reason == UNSEEN_REASON_HOMOGLYPH ? 2 : spans;
        expect(r.spans[reason - 1] == spans && r.bytes[reason - 1] == total, unseen_reason_name(reason));
    }
    expect(r.at.reason == UNSEEN_REASON_NONE, "no rejection");
    unseen_report_free(&r);
    expect(r.output == NULL && r.edits == NULL && r.edits_len == 0, "report_free clears the lists");
    unseen_report_free(&r);

    static const char refused[] = "a\0\0b";
    memset(&r, 0xA5, sizeof r);
    status = unseen_sanitize_text_report((const uint8_t *)refused, sizeof refused - 1, NULL, NULL, &r);
    expect(status == UNSEEN_MALFORMED && r.status == UNSEEN_MALFORMED, "rejection status");
    expect(fragment(r.at, 1, 3, UNSEEN_REASON_CONTROL, 0), "rejection location");
    unseen_report_free(&r);
}

static void file(void) {
    static const char name[] = "notes.md";
    static const char body[] = "x\xE2\x80\x8B";
    unseen_result r;
    memset(&r, 0xA5, sizeof r);
    uint32_t status = unseen_sanitize_file((const uint8_t *)name, strlen(name), (const uint8_t *)body,
                                           strlen(body), NULL, NULL, &r);
    expect(status == UNSEEN_OK, "sanitize_file status");
    expect(bytes(r.mime, r.mime_len, "text/plain") && bytes(r.ext, r.ext_len, "md"), "sanitize_file type");
    expect(bytes(r.output, r.output_len, "x"), "sanitize_file output");
    unseen_result_free(&r);
}

static bool cancel(void *context) {
    (void)context;
    return true;
}

static void cancellation(void) {
    unseen_result r;
    uint32_t status = unseen_sanitize_text((const uint8_t *)"x", 1, cancel, NULL, &r);
    expect(status == UNSEEN_CANCELLED, "cancel function");
    unseen_result_free(&r);
}

int main(void) {
    statuses();
    reasons();
    text();
    report();
    file();
    cancellation();
    expect(named(unseen_version(), UNSEEN_EXPECTED_VERSION), "unseen_version");
    expect(named(unseen_unicode_version(), "17.0.0"), "unseen_unicode_version");
    uint8_t *p = unseen_alloc(sizeof(unseen_report));
    expect(p != NULL && (uintptr_t)p % _Alignof(unseen_report) == 0, "unseen_alloc alignment");
    unseen_dealloc(p, sizeof(unseen_report));
    if (failures != 0) {
        fprintf(stderr, "header.c: %d checks failed\n", failures);
        return 1;
    }
    return 0;
}
