/* rdp/clipboard_codec.rs
 *
 * Copyright 2026 Florian Richter
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Encoding and decoding of RDP clipboard payloads.

use super::session::{LocalClipboardFile, RemoteClipboardFile};

pub(super) fn encode_clipboard_text(text: &str) -> Vec<u8> {
    let mut data = Vec::with_capacity((text.len() + 1) * 2);
    for unit in text.encode_utf16().chain(std::iter::once(0)) {
        data.extend_from_slice(&unit.to_le_bytes());
    }
    data
}

pub(super) fn decode_clipboard_text(data: &[u8]) -> Option<String> {
    let mut units = Vec::with_capacity(data.len() / 2);
    for chunk in data.chunks_exact(2) {
        let unit = u16::from_le_bytes([chunk[0], chunk[1]]);
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    String::from_utf16(&units).ok()
}

pub(super) fn encode_file_group_descriptor(files: &[LocalClipboardFile]) -> Vec<u8> {
    const FILEDESCRIPTORW_SIZE: usize = 592;
    const FD_ATTRIBUTES: u32 = 0x0000_0004;
    const FD_FILESIZE: u32 = 0x0000_0040;
    const FD_UNICODE: u32 = 0x8000_0000;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;

    let mut data = Vec::with_capacity(4 + files.len() * FILEDESCRIPTORW_SIZE);
    data.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for file in files {
        let start = data.len();
        data.resize(start + FILEDESCRIPTORW_SIZE, 0);
        data[start..start + 4]
            .copy_from_slice(&(FD_UNICODE | FD_ATTRIBUTES | FD_FILESIZE).to_le_bytes());
        let attributes = if file.is_directory {
            FILE_ATTRIBUTE_DIRECTORY
        } else {
            FILE_ATTRIBUTE_NORMAL
        };
        data[start + 36..start + 40].copy_from_slice(&attributes.to_le_bytes());
        data[start + 64..start + 68].copy_from_slice(&((file.size >> 32) as u32).to_le_bytes());
        data[start + 68..start + 72].copy_from_slice(&(file.size as u32).to_le_bytes());
        for (i, unit) in file.name.encode_utf16().take(259).enumerate() {
            let offset = start + 72 + i * 2;
            data[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
        }
    }
    data
}

pub(super) fn decode_file_group_descriptor(data: &[u8]) -> Option<Vec<RemoteClipboardFile>> {
    const FILEDESCRIPTORW_SIZE: usize = 592;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
    if data.len() < 4 {
        return None;
    }
    let count = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    if data.len() < 4 + count.checked_mul(FILEDESCRIPTORW_SIZE)? {
        return None;
    }
    let mut files = Vec::with_capacity(count);
    for i in 0..count {
        let start = 4 + i * FILEDESCRIPTORW_SIZE;
        let attributes = u32::from_le_bytes(data[start + 36..start + 40].try_into().ok()?);
        let size_high = u32::from_le_bytes(data[start + 64..start + 68].try_into().ok()?);
        let size_low = u32::from_le_bytes(data[start + 68..start + 72].try_into().ok()?);
        let mut units = Vec::new();
        for chunk in data[start + 72..start + 72 + 520].chunks_exact(2) {
            let unit = u16::from_le_bytes([chunk[0], chunk[1]]);
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        let name = sanitize_remote_file_name(&String::from_utf16(&units).ok()?)?;
        files.push(RemoteClipboardFile {
            name,
            size: ((size_high as u64) << 32) | size_low as u64,
            is_directory: attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
        });
    }
    Some(files)
}

fn sanitize_remote_file_name(name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    let path = std::path::Path::new(&name);
    let file_name = path.file_name()?.to_str()?.to_owned();
    if file_name.is_empty() || file_name == "." || file_name == ".." {
        None
    } else {
        Some(file_name)
    }
}
