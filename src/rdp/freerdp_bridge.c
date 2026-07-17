/* rdp/freerdp_bridge.c
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

#define _POSIX_C_SOURCE 200809L

#include "freerdp_bridge.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <freerdp/addin.h>
#include <freerdp/client.h>
#include <freerdp/client/channels.h>
#include <freerdp/client/cliprdr.h>
#include <freerdp/client/cmdline.h>
#include <freerdp/client/disp.h>
#include <freerdp/client/file.h>
#include <freerdp/channels/cliprdr.h>
#include <freerdp/channels/rdpgfx.h>
#include <freerdp/codec/color.h>
#include <freerdp/constants.h>
#include <freerdp/error.h>
#include <freerdp/gdi/gdi.h>
#include <freerdp/gdi/gfx.h>
#include <freerdp/graphics.h>
#include <freerdp/input.h>
#include <freerdp/settings.h>
#include <freerdp/channels/disp.h>
#include <winpr/crt.h>
#include <winpr/synch.h>
#include <winpr/user.h>

#define LL_FORMAT_FILEGROUPDESCRIPTORW 0xC0A0
#define LL_FILEGROUPDESCRIPTORW_NAME "FileGroupDescriptorW"

typedef struct {
    rdpClientContext common;
    LLSession* session;
    DispClientContext* disp;
    RdpgfxClientContext* gfx;
    CliprdrClientContext* cliprdr;
} LLContext;

typedef struct {
    rdpPointer pointer;
    uint8_t* data;
} LLPointer;

struct LLSession {
    rdpContext* context;
    LLSessionCallbacks callbacks;
    uint32_t last_error;
    uint8_t* clipboard_text;
    uint32_t clipboard_text_size;
    uint8_t* clipboard_file_descriptor;
    uint32_t clipboard_file_descriptor_size;
    uint32_t clipboard_file_count;
    uint32_t remote_file_descriptor_format;
    uint32_t remote_clip_data_id;
    BOOL remote_has_clip_data_id;
};

static LLContext* ll_context(rdpContext* context)
{
    return (LLContext*)context;
}

static LLSession* ll_from_context(rdpContext* context)
{
    LLContext* ll = ll_context(context);
    return ll ? ll->session : NULL;
}

static BOOL ll_begin_paint(rdpContext* context)
{
    if (!context || !context->gdi || !context->gdi->primary ||
        !context->gdi->primary->hdc || !context->gdi->primary->hdc->hwnd ||
        !context->gdi->primary->hdc->hwnd->invalid)
        return FALSE;
    context->gdi->primary->hdc->hwnd->invalid->null = TRUE;
    return TRUE;
}

static BOOL ll_end_paint(rdpContext* context)
{
    LLSession* session = ll_from_context(context);
    rdpGdi* gdi = context ? context->gdi : NULL;
    if (!session || !gdi || !gdi->primary_buffer)
        return FALSE;
    if (gdi->primary && gdi->primary->hdc && gdi->primary->hdc->hwnd &&
        gdi->primary->hdc->hwnd->invalid && gdi->primary->hdc->hwnd->invalid->null)
        return TRUE;
    if (session->callbacks.frame)
        session->callbacks.frame(session->callbacks.user_data, gdi->primary_buffer,
                                 (uint32_t)gdi->width, (uint32_t)gdi->height, gdi->stride);
    return TRUE;
}

static BOOL ll_desktop_resize(rdpContext* context)
{
    if (!context || !context->gdi || !context->settings)
        return FALSE;
    return gdi_resize(context->gdi,
                      freerdp_settings_get_uint32(context->settings, FreeRDP_DesktopWidth),
                      freerdp_settings_get_uint32(context->settings, FreeRDP_DesktopHeight));
}

static BOOL ll_pointer_new(rdpContext* context, rdpPointer* pointer)
{
    LLPointer* ll = (LLPointer*)pointer;
    if (!context || !context->gdi || !pointer || pointer->width == 0 || pointer->height == 0)
        return FALSE;
    const size_t size = 4ULL * pointer->width * pointer->height;
    ll->data = malloc(size);
    if (!ll->data)
        return FALSE;
    if (!freerdp_image_copy_from_pointer_data(
            ll->data, PIXEL_FORMAT_RGBA32, 0, 0, 0, pointer->width, pointer->height,
            pointer->xorMaskData, pointer->lengthXorMask, pointer->andMaskData,
            pointer->lengthAndMask, pointer->xorBpp, &context->gdi->palette)) {
        free(ll->data);
        ll->data = NULL;
        return FALSE;
    }
    return TRUE;
}

static void ll_pointer_free(rdpContext* context, rdpPointer* pointer)
{
    (void)context;
    if (pointer)
        free(((LLPointer*)pointer)->data);
}

static BOOL ll_pointer_set(rdpContext* context, rdpPointer* pointer)
{
    LLSession* session = ll_from_context(context);
    LLPointer* ll = (LLPointer*)pointer;
    if (!session || !pointer || !ll->data)
        return FALSE;
    if (session->callbacks.cursor)
        session->callbacks.cursor(session->callbacks.user_data, ll->data, pointer->width,
                                  pointer->height, pointer->xPos, pointer->yPos);
    return TRUE;
}

static BOOL ll_pointer_system(rdpContext* context, uint32_t kind)
{
    LLSession* session = ll_from_context(context);
    if (!session)
        return FALSE;
    if (session->callbacks.cursor_system)
        session->callbacks.cursor_system(session->callbacks.user_data, kind);
    return TRUE;
}

static BOOL ll_pointer_null(rdpContext* context)
{
    return ll_pointer_system(context, 0);
}

static BOOL ll_pointer_default(rdpContext* context)
{
    return ll_pointer_system(context, 1);
}

static BOOL ll_pointer_position(rdpContext* context, uint32_t x, uint32_t y)
{
    (void)context;
    (void)x;
    (void)y;
    return TRUE;
}

static UINT ll_cliprdr_send_format_list_response(CliprdrClientContext* cliprdr, UINT16 flags)
{
    if (!cliprdr || !cliprdr->ClientFormatListResponse)
        return ERROR_INTERNAL_ERROR;
    CLIPRDR_FORMAT_LIST_RESPONSE response = { 0 };
    response.common.msgType = CB_FORMAT_LIST_RESPONSE;
    response.common.msgFlags = flags;
    return cliprdr->ClientFormatListResponse(cliprdr, &response);
}

static UINT ll_cliprdr_server_capabilities(CliprdrClientContext* cliprdr,
                                           const CLIPRDR_CAPABILITIES* capabilities)
{
    if (!cliprdr || !cliprdr->ClientCapabilities || !capabilities ||
        capabilities->cCapabilitiesSets < 1 || !capabilities->capabilitySets)
        return ERROR_INTERNAL_ERROR;

    const CLIPRDR_GENERAL_CAPABILITY_SET* server =
        (const CLIPRDR_GENERAL_CAPABILITY_SET*)capabilities->capabilitySets;
    CLIPRDR_GENERAL_CAPABILITY_SET general = *server;
    general.version = CB_CAPS_VERSION_2;
    general.generalFlags |= CB_USE_LONG_FORMAT_NAMES | CB_STREAM_FILECLIP_ENABLED |
                            CB_CAN_LOCK_CLIPDATA;
    general.generalFlags &= ~CB_FILECLIP_NO_FILE_PATHS;

    CLIPRDR_CAPABILITIES client = { 0 };
    client.common.msgType = CB_CLIP_CAPS;
    client.cCapabilitiesSets = 1;
    client.capabilitySets = (CLIPRDR_CAPABILITY_SET*)&general;
    return cliprdr->ClientCapabilities(cliprdr, &client);
}

static UINT ll_cliprdr_monitor_ready(CliprdrClientContext* cliprdr,
                                     const CLIPRDR_MONITOR_READY* monitorReady)
{
    (void)monitorReady;
    if (!cliprdr || !cliprdr->rdpcontext)
        return ERROR_INTERNAL_ERROR;
    LLSession* session = ll_from_context(cliprdr->rdpcontext);
    if (session && session->clipboard_file_descriptor && session->clipboard_file_count > 0)
        return ll_session_clipboard_set_files(session, session->clipboard_file_descriptor,
                                              session->clipboard_file_descriptor_size,
                                              session->clipboard_file_count) ? CHANNEL_RC_OK
                                                                            : ERROR_INTERNAL_ERROR;
    if (session && session->clipboard_text && session->clipboard_text_size > 0)
        return ll_session_clipboard_set_text(session, session->clipboard_text,
                                             session->clipboard_text_size) ? CHANNEL_RC_OK
                                                                          : ERROR_INTERNAL_ERROR;
    return CHANNEL_RC_OK;
}

static UINT ll_cliprdr_server_format_list(CliprdrClientContext* cliprdr,
                                          const CLIPRDR_FORMAT_LIST* formatList)
{
    if (!cliprdr || !cliprdr->rdpcontext || !formatList)
        return ERROR_INTERNAL_ERROR;
    BOOL has_text = FALSE;
    BOOL has_files = FALSE;
    LLSession* session = ll_from_context(cliprdr->rdpcontext);
    if (session) {
        if (session->remote_has_clip_data_id && cliprdr->ClientUnlockClipboardData) {
            CLIPRDR_UNLOCK_CLIPBOARD_DATA unlock = { 0 };
            unlock.common.msgType = CB_UNLOCK_CLIPDATA;
            unlock.clipDataId = session->remote_clip_data_id;
            fprintf(stderr, "longlens: unlocking previous remote clipboard data id=%u\n",
                    (unsigned)unlock.clipDataId);
            cliprdr->ClientUnlockClipboardData(cliprdr, &unlock);
        }
        session->remote_file_descriptor_format = 0;
        session->remote_has_clip_data_id = FALSE;
    }
    fprintf(stderr, "longlens: cliprdr server format list: %u formats\n",
            (unsigned)formatList->numFormats);
    for (UINT32 i = 0; i < formatList->numFormats; i++) {
        const UINT32 format = formatList->formats[i].formatId;
        const char* name = formatList->formats[i].formatName;
        fprintf(stderr, "longlens: cliprdr format[%u]: id=%u name=%s\n", (unsigned)i,
                (unsigned)format, name ? name : "(null)");
        if (format == CF_UNICODETEXT) {
            has_text = TRUE;
        } else if (name && strcmp(name, LL_FILEGROUPDESCRIPTORW_NAME) == 0) {
            has_files = TRUE;
            if (session)
                session->remote_file_descriptor_format = format;
        }
    }
    fprintf(stderr, "longlens: cliprdr format list result: has_files=%d has_text=%d file_format=%u\n",
            has_files, has_text,
            session ? (unsigned)session->remote_file_descriptor_format : 0u);
    UINT status = ll_cliprdr_send_format_list_response(cliprdr, CB_RESPONSE_OK);
    if (has_files) {
        if (session && cliprdr->ClientLockClipboardData) {
            session->remote_clip_data_id++;
            if (session->remote_clip_data_id == 0)
                session->remote_clip_data_id = 1;
            CLIPRDR_LOCK_CLIPBOARD_DATA lock = { 0 };
            lock.common.msgType = CB_LOCK_CLIPDATA;
            lock.clipDataId = session->remote_clip_data_id;
            fprintf(stderr, "longlens: locking remote clipboard data id=%u\n",
                    (unsigned)lock.clipDataId);
            const UINT lock_status = cliprdr->ClientLockClipboardData(cliprdr, &lock);
            fprintf(stderr, "longlens: remote clipboard data lock status=%u\n",
                    (unsigned)lock_status);
            session->remote_has_clip_data_id = lock_status == CHANNEL_RC_OK;
        }
        if (session && session->remote_file_descriptor_format != 0 &&
            cliprdr->ClientFormatDataRequest) {
            CLIPRDR_FORMAT_DATA_REQUEST request = { 0 };
            request.common.msgType = CB_FORMAT_DATA_REQUEST;
            request.requestedFormatId = session->remote_file_descriptor_format;
            cliprdr->lastRequestedFormatId = request.requestedFormatId;
            fprintf(stderr,
                    "longlens: immediately requesting remote FileGroupDescriptorW format id=%u\n",
                    (unsigned)request.requestedFormatId);
            const UINT request_status = cliprdr->ClientFormatDataRequest(cliprdr, &request);
            fprintf(stderr,
                    "longlens: immediate remote FileGroupDescriptorW request status=%u\n",
                    (unsigned)request_status);
        } else if (session && session->callbacks.clipboard_offer_files) {
            session->callbacks.clipboard_offer_files(session->callbacks.user_data);
        }
    } else if (has_text) {
        if (cliprdr->ClientFormatDataRequest) {
            CLIPRDR_FORMAT_DATA_REQUEST request = { 0 };
            request.common.msgType = CB_FORMAT_DATA_REQUEST;
            request.requestedFormatId = CF_UNICODETEXT;
            cliprdr->lastRequestedFormatId = request.requestedFormatId;
            fprintf(stderr, "longlens: immediately requesting remote text clipboard\n");
            const UINT request_status = cliprdr->ClientFormatDataRequest(cliprdr, &request);
            fprintf(stderr, "longlens: immediate remote text request status=%u\n",
                    (unsigned)request_status);
        } else if (session && session->callbacks.clipboard_offer_text) {
            session->callbacks.clipboard_offer_text(session->callbacks.user_data);
        }
    }
    return status;
}

static UINT ll_cliprdr_server_format_data_request(
    CliprdrClientContext* cliprdr, const CLIPRDR_FORMAT_DATA_REQUEST* request)
{
    if (!cliprdr || !cliprdr->rdpcontext || !cliprdr->ClientFormatDataResponse || !request)
        return ERROR_INTERNAL_ERROR;

    LLSession* session = ll_from_context(cliprdr->rdpcontext);
    CLIPRDR_FORMAT_DATA_RESPONSE response = { 0 };
    response.common.msgType = CB_FORMAT_DATA_RESPONSE;
    if (session && request->requestedFormatId == CF_UNICODETEXT && session->clipboard_text) {
        response.common.msgFlags = CB_RESPONSE_OK;
        response.common.dataLen = session->clipboard_text_size;
        response.requestedFormatData = session->clipboard_text;
    } else if (session && request->requestedFormatId == LL_FORMAT_FILEGROUPDESCRIPTORW &&
               session->clipboard_file_descriptor) {
        response.common.msgFlags = CB_RESPONSE_OK;
        response.common.dataLen = session->clipboard_file_descriptor_size;
        response.requestedFormatData = session->clipboard_file_descriptor;
    } else {
        response.common.msgFlags = CB_RESPONSE_FAIL;
    }
    return cliprdr->ClientFormatDataResponse(cliprdr, &response);
}

static UINT ll_cliprdr_server_file_contents_request(
    CliprdrClientContext* cliprdr, const CLIPRDR_FILE_CONTENTS_REQUEST* request)
{
    if (!cliprdr || !cliprdr->rdpcontext || !cliprdr->ClientFileContentsResponse || !request)
        return ERROR_INTERNAL_ERROR;
    LLSession* session = ll_from_context(cliprdr->rdpcontext);
    CLIPRDR_FILE_CONTENTS_RESPONSE response = { 0 };
    response.common.msgType = CB_FILECONTENTS_RESPONSE;
    response.streamId = request->streamId;

    uint8_t size_buffer[8] = { 0 };
    uint8_t* data = NULL;
    uint32_t data_size = 0;
    BOOL ok = FALSE;

    if (session && request->listIndex < session->clipboard_file_count) {
        if (request->dwFlags & FILECONTENTS_SIZE) {
            const uint64_t size = session->callbacks.clipboard_file_size
                                      ? session->callbacks.clipboard_file_size(
                                            session->callbacks.user_data, request->listIndex)
                                      : 0;
            memcpy(size_buffer, &size, sizeof(size));
            data = size_buffer;
            data_size = sizeof(size_buffer);
            ok = TRUE;
        } else if ((request->dwFlags & FILECONTENTS_RANGE) && request->cbRequested > 0 &&
                   session->callbacks.clipboard_file_contents) {
            data_size = request->cbRequested;
            data = malloc(data_size);
            if (data) {
                const uint64_t offset = ((uint64_t)request->nPositionHigh << 32) |
                                        (uint64_t)request->nPositionLow;
                data_size = session->callbacks.clipboard_file_contents(
                    session->callbacks.user_data, request->listIndex, offset, data, data_size);
                ok = TRUE;
            }
        }
    }

    response.common.msgFlags = ok ? CB_RESPONSE_OK : CB_RESPONSE_FAIL;
    response.common.dataLen = ok ? data_size : 0;
    response.cbRequested = ok ? data_size : 0;
    response.requestedData = ok ? data : NULL;
    UINT status = cliprdr->ClientFileContentsResponse(cliprdr, &response);
    if (data && data != size_buffer)
        free(data);
    return status;
}

static UINT ll_cliprdr_server_file_contents_response(
    CliprdrClientContext* cliprdr, const CLIPRDR_FILE_CONTENTS_RESPONSE* response)
{
    if (!cliprdr || !cliprdr->rdpcontext || !response)
        return ERROR_INTERNAL_ERROR;
    LLSession* session = ll_from_context(cliprdr->rdpcontext);
    fprintf(stderr,
            "longlens: cliprdr file contents response: flags=0x%04x stream=%u dataLen=%u cbRequested=%u\n",
            (unsigned)response->common.msgFlags, (unsigned)response->streamId,
            (unsigned)response->common.dataLen, (unsigned)response->cbRequested);
    if (session && session->callbacks.clipboard_file_contents_response &&
        (response->common.msgFlags & CB_RESPONSE_OK) && response->requestedData &&
        response->cbRequested > 0) {
        session->callbacks.clipboard_file_contents_response(
            session->callbacks.user_data, response->streamId, response->requestedData,
            response->cbRequested);
    }
    return CHANNEL_RC_OK;
}

static UINT ll_cliprdr_server_format_data_response(
    CliprdrClientContext* cliprdr, const CLIPRDR_FORMAT_DATA_RESPONSE* response)
{
    if (!cliprdr || !cliprdr->rdpcontext || !response)
        return ERROR_INTERNAL_ERROR;
    LLSession* session = ll_from_context(cliprdr->rdpcontext);
    fprintf(stderr,
            "longlens: cliprdr format data response: flags=0x%04x len=%u last_requested=%u remote_file_format=%u\n",
            (unsigned)response->common.msgFlags, (unsigned)response->common.dataLen,
            (unsigned)cliprdr->lastRequestedFormatId,
            session ? (unsigned)session->remote_file_descriptor_format : 0u);
    if (session && (response->common.msgFlags & CB_RESPONSE_OK) && response->requestedFormatData &&
        response->common.dataLen > 0) {
        if (cliprdr->lastRequestedFormatId == CF_UNICODETEXT && session->callbacks.clipboard_text) {
            session->callbacks.clipboard_text(session->callbacks.user_data,
                                              response->requestedFormatData,
                                              response->common.dataLen);
        } else if (cliprdr->lastRequestedFormatId == session->remote_file_descriptor_format &&
                   session->callbacks.clipboard_files) {
            session->callbacks.clipboard_files(session->callbacks.user_data,
                                               response->requestedFormatData,
                                               response->common.dataLen);
        }
    }
    return CHANNEL_RC_OK;
}

static void ll_channel_connected(void* data, const ChannelConnectedEventArgs* event)
{
    LLContext* context = data;
    if (!context || !event)
        return;
    if (strcmp(event->name, DISP_DVC_CHANNEL_NAME) == 0) {
        context->disp = (DispClientContext*)event->pInterface;
    } else if (strcmp(event->name, CLIPRDR_SVC_CHANNEL_NAME) == 0) {
        context->cliprdr = (CliprdrClientContext*)event->pInterface;
        if (context->cliprdr) {
            context->cliprdr->ServerCapabilities = ll_cliprdr_server_capabilities;
            context->cliprdr->MonitorReady = ll_cliprdr_monitor_ready;
            context->cliprdr->ServerFormatList = ll_cliprdr_server_format_list;
            context->cliprdr->ServerFormatDataRequest = ll_cliprdr_server_format_data_request;
            context->cliprdr->ServerFormatDataResponse = ll_cliprdr_server_format_data_response;
            context->cliprdr->ServerFileContentsRequest = ll_cliprdr_server_file_contents_request;
            context->cliprdr->ServerFileContentsResponse = ll_cliprdr_server_file_contents_response;
        }
    } else if (strcmp(event->name, RDPGFX_DVC_CHANNEL_NAME) == 0) {
        context->gfx = (RdpgfxClientContext*)event->pInterface;
        if (context->common.context.gdi)
            gdi_graphics_pipeline_init(context->common.context.gdi, context->gfx);
    } else {
        freerdp_client_OnChannelConnectedEventHandler(data, event);
    }
}

static void ll_channel_disconnected(void* data, const ChannelDisconnectedEventArgs* event)
{
    LLContext* context = data;
    if (!context || !event)
        return;
    if (strcmp(event->name, DISP_DVC_CHANNEL_NAME) == 0) {
        context->disp = NULL;
    } else if (strcmp(event->name, CLIPRDR_SVC_CHANNEL_NAME) == 0) {
        context->cliprdr = NULL;
    } else if (strcmp(event->name, RDPGFX_DVC_CHANNEL_NAME) == 0) {
        if (context->common.context.gdi && context->gfx)
            gdi_graphics_pipeline_uninit(context->common.context.gdi, context->gfx);
        context->gfx = NULL;
    } else {
        freerdp_client_OnChannelDisconnectedEventHandler(data, event);
    }
}

static BOOL ll_pre_connect(freerdp* instance)
{
    if (!instance || !instance->context || !instance->context->pubSub)
        return FALSE;
    if (PubSub_SubscribeChannelConnected(instance->context->pubSub, ll_channel_connected) < 0)
        return FALSE;
    if (PubSub_SubscribeChannelDisconnected(instance->context->pubSub,
                                             ll_channel_disconnected) < 0)
        return FALSE;
    if (!freerdp_client_load_addins(instance->context->channels, instance->context->settings))
        return FALSE;
    return TRUE;
}

static BOOL ll_post_connect(freerdp* instance)
{
    if (!instance || !instance->context || !instance->context->update)
        return FALSE;
    if (!gdi_init(instance, PIXEL_FORMAT_BGRX32))
        return FALSE;

    rdpContext* context = instance->context;
    context->update->BeginPaint = ll_begin_paint;
    context->update->EndPaint = ll_end_paint;
    context->update->DesktopResize = ll_desktop_resize;

    rdpPointer pointer = { 0 };
    pointer.size = sizeof(LLPointer);
    pointer.New = ll_pointer_new;
    pointer.Free = ll_pointer_free;
    pointer.Set = ll_pointer_set;
    pointer.SetNull = ll_pointer_null;
    pointer.SetDefault = ll_pointer_default;
    pointer.SetPosition = ll_pointer_position;
    graphics_register_pointer(context->graphics, &pointer);
    return TRUE;
}

static void ll_post_disconnect(freerdp* instance)
{
    if (!instance || !instance->context)
        return;
    PubSub_UnsubscribeChannelConnected(instance->context->pubSub, ll_channel_connected);
    PubSub_UnsubscribeChannelDisconnected(instance->context->pubSub, ll_channel_disconnected);
    if (instance->context->gdi)
        gdi_free(instance);
}

static DWORD ll_verify_certificate(
    freerdp* instance, const char* host, UINT16 port, const char* common_name,
    const char* subject, const char* issuer, const char* fingerprint, DWORD flags)
{
    LLSession* session = instance && instance->context ? ll_from_context(instance->context) : NULL;
    if (!session || !session->callbacks.verify_certificate)
        return 0;
    return session->callbacks.verify_certificate(
        session->callbacks.user_data, host, port, common_name, subject, issuer, fingerprint,
        flags, NULL, NULL, NULL);
}

static DWORD ll_verify_changed_certificate(
    freerdp* instance, const char* host, UINT16 port, const char* common_name,
    const char* subject, const char* issuer, const char* fingerprint,
    const char* old_subject, const char* old_issuer, const char* old_fingerprint, DWORD flags)
{
    LLSession* session = instance && instance->context ? ll_from_context(instance->context) : NULL;
    if (!session || !session->callbacks.verify_certificate)
        return 0;
    return session->callbacks.verify_certificate(
        session->callbacks.user_data, host, port, common_name, subject, issuer, fingerprint,
        flags, old_subject, old_issuer, old_fingerprint);
}

static BOOL ll_client_new(freerdp* instance, rdpContext* context)
{
    if (!instance || !context)
        return FALSE;
    instance->PreConnect = ll_pre_connect;
    instance->PostConnect = ll_post_connect;
    instance->PostDisconnect = ll_post_disconnect;
    instance->VerifyCertificateEx = ll_verify_certificate;
    instance->VerifyChangedCertificateEx = ll_verify_changed_certificate;
    return TRUE;
}

static void ll_client_free(freerdp* instance, rdpContext* context)
{
    (void)instance;
    (void)context;
}

static int ll_client_start(rdpContext* context)
{
    (void)context;
    return 0;
}

static int ll_client_stop(rdpContext* context)
{
    (void)context;
    return 0;
}

static BOOL ll_set_string(rdpSettings* settings, FreeRDP_Settings_Keys_String key,
                          const char* value)
{
    return freerdp_settings_set_string(settings, key, value ? value : "");
}

LLSession* ll_session_new(const LLSessionConfig* config, const LLSessionCallbacks* callbacks)
{
    if (!config || !callbacks || !config->hostname)
        return NULL;

    RDP_CLIENT_ENTRY_POINTS entry = { 0 };
    entry.Version = RDP_CLIENT_INTERFACE_VERSION;
    entry.Size = sizeof(entry);
    entry.ContextSize = sizeof(LLContext);
    entry.ClientNew = ll_client_new;
    entry.ClientFree = ll_client_free;
    entry.ClientStart = ll_client_start;
    entry.ClientStop = ll_client_stop;

    rdpContext* context = freerdp_client_context_new(&entry);
    if (!context)
        return NULL;

    LLSession* session = calloc(1, sizeof(*session));
    if (!session) {
        freerdp_client_context_free(context);
        return NULL;
    }
    session->context = context;
    session->callbacks = *callbacks;
    ll_context(context)->session = session;

    rdpSettings* settings = context->settings;
    BOOL ok =
        ll_set_string(settings, FreeRDP_ServerHostname, config->hostname) &&
        freerdp_settings_set_uint32(settings, FreeRDP_ServerPort, config->port) &&
        ll_set_string(settings, FreeRDP_Username, config->username) &&
        ll_set_string(settings, FreeRDP_Domain, config->domain) &&
        ll_set_string(settings, FreeRDP_Password, config->password) &&
        ll_set_string(settings, FreeRDP_ConfigPath, config->config_path) &&
        freerdp_settings_set_uint32(settings, FreeRDP_DesktopWidth, config->width) &&
        freerdp_settings_set_uint32(settings, FreeRDP_DesktopHeight, config->height) &&
        freerdp_settings_set_uint32(settings, FreeRDP_ColorDepth, 32) &&
        freerdp_settings_set_uint32(settings, FreeRDP_DesktopScaleFactor,
                                    config->desktop_scale) &&
        freerdp_settings_set_uint32(settings, FreeRDP_DeviceScaleFactor, 100) &&
        freerdp_settings_set_bool(settings, FreeRDP_AutoLogonEnabled, TRUE) &&
        freerdp_settings_set_bool(settings, FreeRDP_RedirectClipboard, TRUE) &&
        freerdp_settings_set_uint32(settings, FreeRDP_ClipboardFeatureMask,
                                    CLIPRDR_FLAG_DEFAULT_MASK) &&
        freerdp_settings_set_bool(settings, FreeRDP_AudioPlayback, TRUE) &&
        freerdp_settings_set_bool(settings, FreeRDP_NetworkAutoDetect, TRUE) &&
        freerdp_settings_set_bool(settings, FreeRDP_SupportGraphicsPipeline, TRUE) &&
        freerdp_settings_set_bool(settings, FreeRDP_SupportDisplayControl, TRUE) &&
        freerdp_settings_set_bool(settings, FreeRDP_DynamicResolutionUpdate, TRUE) &&
        freerdp_settings_set_bool(settings, FreeRDP_GfxH264, TRUE) &&
        freerdp_settings_set_bool(settings, FreeRDP_GfxAVC444, TRUE) &&
        freerdp_settings_set_bool(settings, FreeRDP_GfxThinClient, TRUE) &&
        freerdp_settings_set_bool(settings, FreeRDP_GfxSmallCache, TRUE) &&
        freerdp_settings_set_uint32(settings, FreeRDP_OsMajorType, OSMAJORTYPE_UNIX) &&
        freerdp_settings_set_uint32(settings, FreeRDP_OsMinorType, OSMINORTYPE_NATIVE_WAYLAND);

    if (!ok) {
        ll_session_free(session);
        return NULL;
    }
    return session;
}

void ll_session_free(LLSession* session)
{
    if (!session)
        return;
    if (session->context) {
        LLContext* context = ll_context(session->context);
        if (context && context->cliprdr && session->remote_has_clip_data_id &&
            context->cliprdr->ClientUnlockClipboardData) {
            CLIPRDR_UNLOCK_CLIPBOARD_DATA unlock = { 0 };
            unlock.common.msgType = CB_UNLOCK_CLIPDATA;
            unlock.clipDataId = session->remote_clip_data_id;
            context->cliprdr->ClientUnlockClipboardData(context->cliprdr, &unlock);
        }
        freerdp_client_context_free(session->context);
    }
    free(session->clipboard_text);
    free(session->clipboard_file_descriptor);
    free(session);
}

int ll_session_connect(LLSession* session)
{
    if (!session || !session->context)
        return 0;
    const BOOL ok = freerdp_connect(session->context->instance);
    session->last_error = freerdp_get_last_error(session->context);
    return ok ? 1 : 0;
}

int ll_session_poll(LLSession* session, uint32_t timeout_ms)
{
    if (!session || !session->context)
        return -1;
    if (freerdp_shall_disconnect_context(session->context))
        return 0;

    HANDLE handles[MAXIMUM_WAIT_OBJECTS] = { 0 };
    const DWORD count = freerdp_get_event_handles(session->context, handles, MAXIMUM_WAIT_OBJECTS);
    if (count == 0)
        return -1;
    const DWORD status = WaitForMultipleObjects(count, handles, FALSE, timeout_ms);
    if (status == WAIT_FAILED)
        return -1;
    if (status != WAIT_TIMEOUT && !freerdp_check_event_handles(session->context)) {
        session->last_error = freerdp_get_last_error(session->context);
        return -1;
    }
    return 1;
}

void ll_session_disconnect(LLSession* session)
{
    if (session && session->context)
        freerdp_disconnect(session->context->instance);
}

void ll_session_abort(LLSession* session)
{
    if (session && session->context)
        freerdp_abort_connect_context(session->context);
}

uint32_t ll_session_last_error(const LLSession* session)
{
    return session ? session->last_error : FREERDP_ERROR_CONNECT_UNDEFINED;
}

const char* ll_error_name(uint32_t code)
{
    return freerdp_get_last_error_name(code);
}

const char* ll_error_string(uint32_t code)
{
    return freerdp_get_last_error_string(code);
}

uint32_t ll_error_class(uint32_t code)
{
    switch (code) {
        case FREERDP_ERROR_AUTHENTICATION_FAILED:
        case FREERDP_ERROR_CONNECT_LOGON_FAILURE:
        case FREERDP_ERROR_CONNECT_WRONG_PASSWORD:
        case FREERDP_ERROR_CONNECT_NO_OR_MISSING_CREDENTIALS:
            return 1;
        case FREERDP_ERROR_CONNECT_ACCESS_DENIED:
        case FREERDP_ERROR_CONNECT_ACCOUNT_RESTRICTION:
        case FREERDP_ERROR_CONNECT_ACCOUNT_LOCKED_OUT:
        case FREERDP_ERROR_CONNECT_ACCOUNT_EXPIRED:
        case FREERDP_ERROR_CONNECT_LOGON_TYPE_NOT_GRANTED:
            return 2;
        default:
            return 0;
    }
}

int ll_session_send_key(LLSession* session, uint32_t scancode, int pressed)
{
    if (!session || !session->context || !session->context->input)
        return 0;
    return freerdp_input_send_keyboard_event_ex(session->context->input, pressed ? TRUE : FALSE,
                                                FALSE, scancode);
}

int ll_session_send_unicode(LLSession* session, uint16_t code, int pressed)
{
    if (!session || !session->context || !session->context->input)
        return 0;
    return freerdp_input_send_unicode_keyboard_event(
        session->context->input, pressed ? 0 : KBD_FLAGS_RELEASE, code);
}

int ll_session_send_mouse(LLSession* session, uint16_t flags, uint16_t x, uint16_t y)
{
    if (!session || !session->context || !session->context->input)
        return 0;
    return freerdp_input_send_mouse_event(session->context->input, flags, x, y);
}

int ll_session_resize(LLSession* session, uint32_t width, uint32_t height,
                      uint32_t desktop_scale)
{
    if (!session || !session->context)
        return 0;
    LLContext* context = ll_context(session->context);
    if (!context->disp || !context->disp->SendMonitorLayout)
        return 0;

    DISPLAY_CONTROL_MONITOR_LAYOUT layout = { 0 };
    layout.Flags = DISPLAY_CONTROL_MONITOR_PRIMARY;
    layout.Width = width;
    layout.Height = height;
    layout.DesktopScaleFactor = desktop_scale;
    layout.DeviceScaleFactor = 100;
    return context->disp->SendMonitorLayout(context->disp, 1, &layout) == CHANNEL_RC_OK;
}

int ll_session_clipboard_set_text(LLSession* session, const uint8_t* data, uint32_t size)
{
    if (!session)
        return 0;

    uint8_t* copy = NULL;
    if (data && size > 0) {
        copy = malloc(size);
        if (!copy)
            return 0;
        memcpy(copy, data, size);
    }
    free(session->clipboard_text);
    session->clipboard_text = copy;
    session->clipboard_text_size = copy ? size : 0;
    free(session->clipboard_file_descriptor);
    session->clipboard_file_descriptor = NULL;
    session->clipboard_file_descriptor_size = 0;
    session->clipboard_file_count = 0;

    if (!session->context)
        return 0;
    LLContext* context = ll_context(session->context);
    if (!context->cliprdr || !context->cliprdr->ClientFormatList)
        return 0;

    CLIPRDR_FORMAT format = { 0 };
    format.formatId = CF_UNICODETEXT;
    CLIPRDR_FORMAT_LIST list = { 0 };
    list.common.msgType = CB_FORMAT_LIST;
    list.numFormats = copy ? 1 : 0;
    list.formats = copy ? &format : NULL;
    return context->cliprdr->ClientFormatList(context->cliprdr, &list) == CHANNEL_RC_OK;
}

int ll_session_clipboard_set_files(LLSession* session, const uint8_t* descriptor, uint32_t size,
                                   uint32_t count)
{
    if (!session)
        return 0;

    uint8_t* copy = NULL;
    if (descriptor && size > 0 && count > 0) {
        copy = malloc(size);
        if (!copy)
            return 0;
        memcpy(copy, descriptor, size);
    }
    free(session->clipboard_file_descriptor);
    session->clipboard_file_descriptor = copy;
    session->clipboard_file_descriptor_size = copy ? size : 0;
    session->clipboard_file_count = copy ? count : 0;
    free(session->clipboard_text);
    session->clipboard_text = NULL;
    session->clipboard_text_size = 0;

    if (!session->context)
        return 0;
    LLContext* context = ll_context(session->context);
    if (!context->cliprdr || !context->cliprdr->ClientFormatList)
        return 0;

    CLIPRDR_FORMAT formats[1] = { 0 };
    formats[0].formatId = LL_FORMAT_FILEGROUPDESCRIPTORW;
    formats[0].formatName = LL_FILEGROUPDESCRIPTORW_NAME;
    CLIPRDR_FORMAT_LIST list = { 0 };
    list.common.msgType = CB_FORMAT_LIST;
    list.numFormats = copy ? 1 : 0;
    list.formats = copy ? formats : NULL;
    return context->cliprdr->ClientFormatList(context->cliprdr, &list) == CHANNEL_RC_OK;
}

int ll_session_clipboard_request_text(LLSession* session)
{
    if (!session || !session->context)
        return 0;
    LLContext* context = ll_context(session->context);
    if (!context->cliprdr || !context->cliprdr->ClientFormatDataRequest)
        return 0;

    CLIPRDR_FORMAT_DATA_REQUEST request = { 0 };
    request.common.msgType = CB_FORMAT_DATA_REQUEST;
    request.requestedFormatId = CF_UNICODETEXT;
    context->cliprdr->lastRequestedFormatId = request.requestedFormatId;
    const UINT status = context->cliprdr->ClientFormatDataRequest(context->cliprdr, &request);
    fprintf(stderr, "longlens: requested remote text clipboard status=%u\n", (unsigned)status);
    return status == CHANNEL_RC_OK;
}

int ll_session_clipboard_request_files(LLSession* session)
{
    if (!session || !session->context || session->remote_file_descriptor_format == 0)
        return 0;
    LLContext* context = ll_context(session->context);
    if (!context->cliprdr || !context->cliprdr->ClientFormatDataRequest)
        return 0;

    CLIPRDR_FORMAT_DATA_REQUEST request = { 0 };
    request.common.msgType = CB_FORMAT_DATA_REQUEST;
    request.requestedFormatId = session->remote_file_descriptor_format;
    context->cliprdr->lastRequestedFormatId = request.requestedFormatId;
    fprintf(stderr, "longlens: requesting remote FileGroupDescriptorW format id=%u\n",
            (unsigned)request.requestedFormatId);
    const UINT status = context->cliprdr->ClientFormatDataRequest(context->cliprdr, &request);
    fprintf(stderr, "longlens: requested remote FileGroupDescriptorW status=%u\n",
            (unsigned)status);
    return status == CHANNEL_RC_OK;
}

int ll_session_clipboard_unlock_remote_files(LLSession* session)
{
    if (!session || !session->context || !session->remote_has_clip_data_id)
        return 1;
    LLContext* context = ll_context(session->context);
    if (!context->cliprdr || !context->cliprdr->ClientUnlockClipboardData)
        return 0;

    CLIPRDR_UNLOCK_CLIPBOARD_DATA unlock = { 0 };
    unlock.common.msgType = CB_UNLOCK_CLIPDATA;
    unlock.clipDataId = session->remote_clip_data_id;
    fprintf(stderr, "longlens: unlocking remote clipboard data id=%u after file transfer\n",
            (unsigned)unlock.clipDataId);
    const UINT status = context->cliprdr->ClientUnlockClipboardData(context->cliprdr, &unlock);
    fprintf(stderr, "longlens: remote clipboard data unlock status=%u\n", (unsigned)status);
    if (status == CHANNEL_RC_OK)
        session->remote_has_clip_data_id = FALSE;
    return status == CHANNEL_RC_OK;
}

int ll_session_clipboard_request_file_size(LLSession* session, uint32_t stream_id,
                                           uint32_t index)
{
    if (!session || !session->context)
        return 0;
    LLContext* context = ll_context(session->context);
    if (!context->cliprdr || !context->cliprdr->ClientFileContentsRequest)
        return 0;

    CLIPRDR_FILE_CONTENTS_REQUEST request = { 0 };
    request.common.msgType = CB_FILECONTENTS_REQUEST;
    request.streamId = stream_id;
    request.listIndex = index;
    request.dwFlags = FILECONTENTS_SIZE;
    request.cbRequested = 8;
    request.haveClipDataId = session->remote_has_clip_data_id;
    request.clipDataId = session->remote_clip_data_id;
    fprintf(stderr, "longlens: requesting remote file size stream=%u index=%u clipDataId=%s/%u\n",
            (unsigned)stream_id, (unsigned)index,
            request.haveClipDataId ? "yes" : "no", (unsigned)request.clipDataId);
    const UINT status = context->cliprdr->ClientFileContentsRequest(context->cliprdr, &request);
    fprintf(stderr, "longlens: requested remote file size status=%u\n", (unsigned)status);
    return status == CHANNEL_RC_OK;
}

int ll_session_clipboard_request_file_contents(LLSession* session, uint32_t stream_id,
                                               uint32_t index, uint64_t offset, uint32_t size)
{
    if (!session || !session->context)
        return 0;
    LLContext* context = ll_context(session->context);
    if (!context->cliprdr || !context->cliprdr->ClientFileContentsRequest)
        return 0;

    CLIPRDR_FILE_CONTENTS_REQUEST request = { 0 };
    request.common.msgType = CB_FILECONTENTS_REQUEST;
    request.streamId = stream_id;
    request.listIndex = index;
    request.dwFlags = FILECONTENTS_RANGE;
    request.nPositionLow = (uint32_t)(offset & 0xffffffffu);
    request.nPositionHigh = (uint32_t)(offset >> 32);
    request.cbRequested = size;
    request.haveClipDataId = session->remote_has_clip_data_id;
    request.clipDataId = session->remote_clip_data_id;
    fprintf(stderr,
            "longlens: requesting remote file contents stream=%u index=%u offset=%llu size=%u clipDataId=%s/%u\n",
            (unsigned)stream_id, (unsigned)index, (unsigned long long)offset, (unsigned)size,
            request.haveClipDataId ? "yes" : "no", (unsigned)request.clipDataId);
    const UINT status = context->cliprdr->ClientFileContentsRequest(context->cliprdr, &request);
    fprintf(stderr, "longlens: requested remote file contents status=%u\n", (unsigned)status);
    return status == CHANNEL_RC_OK;
}

int ll_rdp_file_parse(const char* path, LLRdpFile* result)
{
    if (!path || !result)
        return 0;
    memset(result, 0, sizeof(*result));
    rdpFile* file = freerdp_client_rdp_file_new();
    if (!file)
        return 0;
    if (!freerdp_client_parse_rdp_file(file, path)) {
        freerdp_client_rdp_file_free(file);
        return 0;
    }

    const char* host = freerdp_client_rdp_file_get_string_option(file, "full address");
    const char* user = freerdp_client_rdp_file_get_string_option(file, "username");
    const char* domain = freerdp_client_rdp_file_get_string_option(file, "domain");
    if (host && host[0] != '\0') {
        result->hostname = strdup(host);
        result->username = strdup(user ? user : "");
        result->domain = strdup(domain ? domain : "");
        const int port = freerdp_client_rdp_file_get_integer_option(file, "server port");
        result->port = port > 0 && port <= UINT16_MAX ? (uint16_t)port : 3389;
    }
    freerdp_client_rdp_file_free(file);
    if (!result->hostname || !result->username || !result->domain) {
        ll_rdp_file_clear(result);
        return 0;
    }
    return 1;
}

void ll_rdp_file_clear(LLRdpFile* result)
{
    if (!result)
        return;
    free(result->hostname);
    free(result->username);
    free(result->domain);
    memset(result, 0, sizeof(*result));
}
