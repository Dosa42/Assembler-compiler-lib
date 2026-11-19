/* SPDX-License-Identifier: MIT */
#ifndef BROKKR_USB_CLIENT_H
#define BROKKR_USB_CLIENT_H

#include <stddef.h>

#define BROKKR_USB_DEFAULT_SOCKET "/data/data/com.termux/files/usr/tmp/brokkr-usb/socket"
#define BROKKR_USB_METADATA_CAPACITY 8192

#ifdef __cplusplus
extern "C" {
#endif

/* Returns an owned CLOEXEC descriptor, or -1 with errno; JSON records the broker instance. */
int brokkr_usb_acquire(const char *socket_path, char *metadata_json, size_t capacity);
/* Live RPC only: returns 0 on success, -1 with errno; no cached metadata is trusted. */
int brokkr_usb_status(const char *socket_path, char *metadata_json, size_t capacity);
/* Stops only the broker whose session token matches; existing client copies remain open. */
int brokkr_usb_stop(const char *socket_path, const char *session,
                    char *metadata_json, size_t capacity);

#ifdef __cplusplus
}
#endif
#endif
