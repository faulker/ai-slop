/*
extern "C" surface of the aetr COFDM modem shim.

Fixed configuration: 48000 Hz mono f32, ~1600 Hz OFDM bandwidth centered at
1500 Hz, QPSK payload symbols, CA-SCL polar coding. Three payload modes:
mode 0 = 85 bytes, mode 1 = 128 bytes, mode 2 = 170 bytes per burst.
*/

#pragma once

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Status codes returned by aetr_modem_rx_feed. */
#define AETR_RX_IDLE 0    /* nothing new */
#define AETR_RX_SYNCED 1  /* preamble decoded, payload symbols in flight */
#define AETR_RX_READY 2   /* a payload is ready; call aetr_modem_rx_fetch */
#define AETR_RX_FAILED 3  /* sync or preamble candidate found but decode failed */

/* Number of payload bytes carried by a burst in the given mode (0/1/2 ->
 * 85/128/170), or -1 for an invalid mode. */
int32_t aetr_modem_payload_bytes(int32_t mode);

/* Number of 48 kHz f32 samples a full burst occupies (constant across modes). */
int32_t aetr_modem_burst_samples(void);

/* Encodes one payload of exactly aetr_modem_payload_bytes(mode) bytes into a
 * complete 48 kHz mono f32 burst (sync + preamble + payload symbols) written
 * to out_pcm. Returns the number of samples written, or a negative value on
 * bad arguments / insufficient out_capacity. */
int32_t aetr_modem_encode(int32_t mode, const uint8_t *payload, int32_t payload_len,
                          float *out_pcm, int32_t out_capacity);

/* Creates a streaming decoder. Mode is auto-detected per burst from the
 * preamble, so one receiver handles all three modes. Returns NULL on
 * allocation failure. */
void *aetr_modem_rx_new(void);

/* Feeds 48 kHz mono f32 samples of any length. Returns the strongest status
 * observed while consuming the buffer (AETR_RX_*), or a negative value on bad
 * arguments. When AETR_RX_READY is returned, fetch before feeding more audio. */
int32_t aetr_modem_rx_feed(void *handle, const float *pcm, int32_t len);

/* Copies the decoded payload into out_payload (capacity must be >= 170) and
 * clears the ready flag. Returns the payload byte count (85/128/170), or a
 * negative value if no payload is pending or decoding failed. */
int32_t aetr_modem_rx_fetch(void *handle, uint8_t *out_payload);

/* Destroys a decoder created by aetr_modem_rx_new. NULL is a no-op. */
void aetr_modem_rx_free(void *handle);

#ifdef __cplusplus
}
#endif
