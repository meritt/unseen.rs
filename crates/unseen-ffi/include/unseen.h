#ifndef UNSEEN_H
#define UNSEEN_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

enum {
    UNSEEN_OK = 0,
    UNSEEN_UNSUPPORTED_TYPE = 1,
    UNSEEN_TOO_LARGE = 2,
    UNSEEN_MALFORMED = 3,
    UNSEEN_VERIFY_FAILED = 4,
    UNSEEN_BINARY = 5,
    UNSEEN_CANCELLED = 6,
    UNSEEN_INVALID_ARGUMENT = 7,
    UNSEEN_INTERNAL = 8,
};

enum {
    UNSEEN_REASON_NONE = 0,
    UNSEEN_REASON_BOM = 1,
    UNSEEN_REASON_NONCHARACTER = 2,
    UNSEEN_REASON_UNASSIGNED = 3,
    UNSEEN_REASON_PRIVATE_USE = 4,
    UNSEEN_REASON_BIDI_CONTROL = 5,
    UNSEEN_REASON_JOIN_CONTROL = 6,
    UNSEEN_REASON_VARIATION_SELECTOR = 7,
    UNSEEN_REASON_FORMAT = 8,
    UNSEEN_REASON_DETACHED_MARK = 9,
    UNSEEN_REASON_TRAILING_SPACE = 10,
    UNSEEN_REASON_HOMOGLYPH = 11,
    UNSEEN_REASON_LINE_BREAK = 12,
    UNSEEN_REASON_INVALID_UTF8 = 13,
    UNSEEN_REASON_CONTROL = 14,
    UNSEEN_REASON_MARK_OVERFLOW = 15,
};

#define UNSEEN_EDIT_REASONS 12

typedef bool (*unseen_cancel_fn)(void *context);

typedef struct unseen_result {
    uint32_t status;
    const uint8_t *mime;
    size_t mime_len;
    const uint8_t *ext;
    size_t ext_len;
    uint8_t *output;
    size_t output_len;
} unseen_result;

uint32_t unseen_sanitize_text(const uint8_t *input, size_t input_len,
                               unseen_cancel_fn cancel, void *context,
                               unseen_result *out);

typedef struct unseen_fragment {
    size_t start;
    size_t end;
    uint32_t reason;
    uint32_t with;
} unseen_fragment;

typedef struct unseen_report {
    uint32_t status;
    uint8_t *output;
    size_t output_len;
    unseen_fragment *edits;
    size_t edits_len;
    bool truncated;
    size_t spans[UNSEEN_EDIT_REASONS];
    size_t bytes[UNSEEN_EDIT_REASONS];
    unseen_fragment at;
} unseen_report;

uint32_t unseen_sanitize_text_report(const uint8_t *input, size_t input_len,
                                     unseen_cancel_fn cancel, void *context,
                                     unseen_report *out);

uint32_t unseen_sanitize_file(const uint8_t *name, size_t name_len,
                               const uint8_t *body, size_t body_len,
                               unseen_cancel_fn cancel, void *context,
                               unseen_result *out);

void unseen_result_free(unseen_result *result);

void unseen_report_free(unseen_report *report);

const char *unseen_status_name(uint32_t status);

const char *unseen_reason_name(uint32_t reason);

const char *unseen_unicode_version(void);

const char *unseen_version(void);

uint8_t *unseen_alloc(size_t len);
void unseen_dealloc(uint8_t *p, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* UNSEEN_H */
