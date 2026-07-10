/* rdp/freerdp_bridge.h
 *
 * Copyright 2026 Florian Richter
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

#ifndef LONGLENS_FREERDP_BRIDGE_H
#define LONGLENS_FREERDP_BRIDGE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct LLSession LLSession;

typedef struct {
    const char* hostname;
    uint16_t port;
    const char* username;
    const char* domain;
    const char* password;
    const char* config_path;
    uint32_t width;
    uint32_t height;
    uint32_t desktop_scale;
    bool sound_enabled;
} LLSessionConfig;

typedef struct {
    void* user_data;
    void (*frame)(void* user_data, const uint8_t* data, uint32_t width, uint32_t height,
                  uint32_t stride);
    void (*cursor)(void* user_data, const uint8_t* data, uint32_t width, uint32_t height,
                   uint32_t hotspot_x, uint32_t hotspot_y);
    void (*cursor_system)(void* user_data, uint32_t kind);
    void (*clipboard_offer_text)(void* user_data);
    void (*clipboard_text)(void* user_data, const uint8_t* data, uint32_t size);
    void (*clipboard_offer_files)(void* user_data);
    void (*clipboard_files)(void* user_data, const uint8_t* data, uint32_t size);
    void (*clipboard_file_contents_response)(void* user_data, uint32_t stream_id,
                                             const uint8_t* data, uint32_t size);
    uint64_t (*clipboard_file_size)(void* user_data, uint32_t index);
    uint32_t (*clipboard_file_contents)(void* user_data, uint32_t index, uint64_t offset,
                                        uint8_t* data, uint32_t size);
    uint32_t (*verify_certificate)(
        void* user_data, const char* host, uint16_t port, const char* common_name,
        const char* subject, const char* issuer, const char* fingerprint, uint32_t flags,
        const char* old_subject, const char* old_issuer, const char* old_fingerprint);
} LLSessionCallbacks;

typedef struct {
    char* hostname;
    char* username;
    char* domain;
    uint16_t port;
} LLRdpFile;

LLSession* ll_session_new(const LLSessionConfig* config, const LLSessionCallbacks* callbacks);
void ll_session_free(LLSession* session);
int ll_session_connect(LLSession* session);
int ll_session_poll(LLSession* session, uint32_t timeout_ms);
void ll_session_disconnect(LLSession* session);
void ll_session_abort(LLSession* session);
uint32_t ll_session_last_error(const LLSession* session);
const char* ll_error_name(uint32_t code);
const char* ll_error_string(uint32_t code);
uint32_t ll_error_class(uint32_t code);

int ll_session_send_key(LLSession* session, uint32_t scancode, int pressed);
int ll_session_send_mouse(LLSession* session, uint16_t flags, uint16_t x, uint16_t y);
int ll_session_resize(LLSession* session, uint32_t width, uint32_t height,
                      uint32_t desktop_scale);
int ll_session_clipboard_set_text(LLSession* session, const uint8_t* data, uint32_t size);
int ll_session_clipboard_set_files(LLSession* session, const uint8_t* descriptor, uint32_t size,
                                   uint32_t count);
int ll_session_clipboard_request_text(LLSession* session);
int ll_session_clipboard_request_files(LLSession* session);
int ll_session_clipboard_unlock_remote_files(LLSession* session);
int ll_session_clipboard_request_file_size(LLSession* session, uint32_t stream_id,
                                           uint32_t index);
int ll_session_clipboard_request_file_contents(LLSession* session, uint32_t stream_id,
                                               uint32_t index, uint64_t offset, uint32_t size);

int ll_rdp_file_parse(const char* path, LLRdpFile* result);
void ll_rdp_file_clear(LLRdpFile* result);

#endif
