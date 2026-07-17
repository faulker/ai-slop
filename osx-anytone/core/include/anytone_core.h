/*
 * anytone_core.h — C ABI of the anytone-core Rust static library.
 *
 * Hand-written to match core/src/ffi.rs (keep the two in sync).
 *
 * Conventions:
 *  - Every `char *` returned by these functions is heap-allocated and MUST be
 *    released with anytone_string_free().
 *  - String-returning functions return NULL on failure; status-returning
 *    functions return 0 on success and -1 on failure. On failure, *err_out
 *    (when err_out is non-NULL) is set to a heap-allocated error message the
 *    caller must free with anytone_string_free().
 *  - All input strings are NUL-terminated UTF-8.
 */

#ifndef ANYTONE_CORE_H
#define ANYTONE_CORE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Progress callback: (blocks_done, blocks_total, user_context). Invoked
 * synchronously on the thread that called the FFI function. */
typedef void (*anytone_progress_cb)(size_t done, size_t total, void *user);

/* Free a string previously returned by any function below. NULL is a no-op. */
void anytone_string_free(char *s);

/* Exact byte length of a full codeplug image for the supported radio. A .bin of
 * any other length is from a different model/firmware version and is
 * incompatible with this build. */
size_t anytone_codeplug_size(void);

/* Enumerate serial ports as a JSON array:
 * [{"name","vid","pid","product","likely_radio"}, ...]. */
char *anytone_ports_json(char **err_out);

/* Identify the radio on `port`; returns the model/version string. */
char *anytone_identify(const char *port, char **err_out);

/* Read the full codeplug from the radio into the file `out_path`. */
int32_t anytone_backup(const char *port, const char *out_path,
                       anytone_progress_cb progress, void *user,
                       char **err_out);

/* Write the codeplug file `in_path` back to the radio. Refuses unless `force`
 * is true; checks the model string; every block is read back and verified. */
int32_t anytone_restore(const char *port, const char *in_path, bool force,
                        anytone_progress_cb progress, void *user,
                        char **err_out);

/* Parse a codeplug .bin offline; returns a JSON object with the active records:
 * {"channels":[...],"zones":[...],"contacts":[...],"group_lists":[...],
 *  "radio_ids":[...]}. See the Rust models for each record's fields. */
char *anytone_dump_json(const char *bin_path, char **err_out);

/* Apply a batch of edits (JSON) to `bin_in`, writing `bin_out` (the paths may
 * be equal). The edits object carries, for each entity family, an update array
 * ("channels"/"zones"/"contacts"/"group_lists"/"radio_ids", each element an
 * index plus optional fields), an "add_*" array (field objects with no index;
 * a record is created and the fields applied), and a "remove_*" array of
 * indices. Operations run update → remove → add. Channel fields include name,
 * rx_frequency_hz, tx_frequency_hz, mode, power, bandwidth, color_code,
 * time_slot, contact_index, radio_id_index, group_list_index. See ffi.rs for
 * the full schema. Edits are verified by re-parsing the output. */
int32_t anytone_apply_edits(const char *bin_in, const char *edits_json,
                            const char *bin_out, char **err_out);

#ifdef __cplusplus
}
#endif

#endif /* ANYTONE_CORE_H */
