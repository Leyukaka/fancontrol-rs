//! Windows storage temperatures via `DeviceIoControl` — **no PowerShell**.
//!
//! Uses `IOCTL_STORAGE_QUERY_PROPERTY` + `StorageDeviceTemperatureProperty`
//! (Windows 10+). Opening `\\.\PhysicalDriveN` usually needs elevation
//! (same as PawnIO for hardware control).

#![cfg(windows)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, StorageDeviceTemperatureProperty,
    IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_PROPERTY_QUERY, STORAGE_TEMPERATURE_DATA_DESCRIPTOR,
    STORAGE_TEMPERATURE_VALUE_NOT_REPORTED,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

/// Probe `\\.\PhysicalDrive0` … for temperatures reported by the storage stack.
pub fn probe_storage_temps() -> Vec<(String, String, f64)> {
    let mut rows = Vec::new();
    for index in 0..16u32 {
        let path = format!(r"\\.\PhysicalDrive{index}");
        let Some((name, temp)) = probe_one_drive(&path, index) else {
            continue;
        };
        if !(0.0..=120.0).contains(&temp) {
            continue;
        }
        rows.push((
            format!("host.ssd{index}"),
            format!("Storage ({name})"),
            temp,
        ));
    }
    rows
}

fn probe_one_drive(path: &str, index: u32) -> Option<(String, f64)> {
    let handle = open_drive(path)?;
    let result = (|| {
        let temp = query_temperature(handle)?;
        let name = query_product_name(handle).unwrap_or_else(|| format!("PhysicalDrive{index}"));
        Some((name, temp))
    })();
    unsafe {
        CloseHandle(handle);
    }
    result
}

fn open_drive(path: &str) -> Option<HANDLE> {
    let wide: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0 as HANDLE,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        None
    } else {
        Some(handle)
    }
}

fn query_temperature(handle: HANDLE) -> Option<f64> {
    let mut query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceTemperatureProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    // Header + room for several STORAGE_TEMPERATURE_INFO entries
    let mut buf = vec![0u8; 512];
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &mut query as *mut _ as *mut _,
            std::mem::size_of_val(&query) as u32,
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    let min = std::mem::size_of::<STORAGE_TEMPERATURE_DATA_DESCRIPTOR>() as u32;
    if ok != 0 && returned >= min {
        let desc = unsafe { &*(buf.as_ptr() as *const STORAGE_TEMPERATURE_DATA_DESCRIPTOR) };
        if desc.InfoCount != 0 {
            let t = desc.TemperatureInfo[0].Temperature;
            // 0x8000 = not reported (STORAGE_TEMPERATURE_VALUE_NOT_REPORTED)
            if (t as u16) != (STORAGE_TEMPERATURE_VALUE_NOT_REPORTED as u16) {
                return Some(f64::from(t));
            }
        }
    }
    // Fallback: some SATA/older drives don't report StorageDeviceTemperatureProperty at
    // all. Try the NVMe health-log page directly (no-op / None for non-NVMe devices).
    query_temperature_nvme_fallback(handle)
}

/// NVMe-only fallback: query the health-info log page directly via the documented
/// `STORAGE_PROTOCOL_SPECIFIC_DATA` / `NVME_HEALTH_INFO_LOG` IOCTL contract, for drives
/// where the primary `StorageDeviceTemperatureProperty` query returns nothing.
/// Composite temperature is reported in Kelvin per the NVMe spec.
fn query_temperature_nvme_fallback(handle: HANDLE) -> Option<f64> {
    use windows_sys::Win32::Storage::Nvme::NVME_HEALTH_INFO_LOG;
    use windows_sys::Win32::System::Ioctl::{
        NVMeDataTypeLogPage, ProtocolTypeNvme, StorageDeviceProtocolSpecificProperty,
        STORAGE_PROTOCOL_DATA_DESCRIPTOR, STORAGE_PROTOCOL_SPECIFIC_DATA,
    };

    const NVME_LOG_PAGE_HEALTH_INFO: u32 = 0x02;

    let header_len = std::mem::size_of::<STORAGE_PROPERTY_QUERY>();
    let protocol_data_len = std::mem::size_of::<STORAGE_PROTOCOL_SPECIFIC_DATA>();
    let health_len = std::mem::size_of::<NVME_HEALTH_INFO_LOG>();
    // STORAGE_PROPERTY_QUERY::AdditionalParameters is a 1-byte placeholder for the
    // protocol-specific data that immediately follows the two u32 header fields.
    let query_offset = header_len - 1;

    // Leave room for the log page after the protocol-specific header (some stacks
    // expect a larger transfer buffer than the bare request structure).
    let mut in_buf = vec![0u8; query_offset + protocol_data_len + health_len];
    // SAFETY: `in_buf` is large enough for the header at offset 0.
    unsafe {
        let q = in_buf.as_mut_ptr() as *mut STORAGE_PROPERTY_QUERY;
        (*q).PropertyId = StorageDeviceProtocolSpecificProperty;
        (*q).QueryType = PropertyStandardQuery;
    }
    // SAFETY: `in_buf` is large enough for STORAGE_PROTOCOL_SPECIFIC_DATA starting at
    // `query_offset`, immediately after the header's two u32 fields.
    unsafe {
        let p = in_buf.as_mut_ptr().add(query_offset) as *mut STORAGE_PROTOCOL_SPECIFIC_DATA;
        (*p).ProtocolType = ProtocolTypeNvme;
        (*p).DataType = NVMeDataTypeLogPage as u32;
        (*p).ProtocolDataRequestValue = NVME_LOG_PAGE_HEALTH_INFO;
        (*p).ProtocolDataRequestSubValue = 0;
        (*p).ProtocolDataOffset = protocol_data_len as u32;
        (*p).ProtocolDataLength = health_len as u32;
    }

    // Descriptor header + protocol data + full health log (aligned generously).
    let out_need = std::mem::size_of::<STORAGE_PROTOCOL_DATA_DESCRIPTOR>() + health_len + 64;
    let mut out = vec![0u8; out_need.max(512)];
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            in_buf.as_mut_ptr() as *mut _,
            in_buf.len() as u32,
            out.as_mut_ptr() as *mut _,
            out.len() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    let desc_header_len = std::mem::size_of::<STORAGE_PROTOCOL_DATA_DESCRIPTOR>() as u32;
    if ok == 0 || returned < desc_header_len {
        return None;
    }
    // SAFETY: `returned >= desc_header_len` was just checked above.
    let desc = unsafe { &*(out.as_ptr() as *const STORAGE_PROTOCOL_DATA_DESCRIPTOR) };
    // The driver reports where the actual log-page bytes start, relative to the start
    // of `ProtocolSpecificData` (not necessarily `size_of::<STORAGE_PROTOCOL_SPECIFIC_DATA>()`)
    // — read it back rather than assuming, per the documented IOCTL contract.
    let protocol_data_start = desc_header_len - protocol_data_len as u32;
    let log_offset = (protocol_data_start + desc.ProtocolSpecificData.ProtocolDataOffset) as usize;
    let log_end = log_offset + std::mem::size_of::<NVME_HEALTH_INFO_LOG>();
    if returned < log_end as u32 {
        return None;
    }
    let log = unsafe { &*(out.as_ptr().add(log_offset) as *const NVME_HEALTH_INFO_LOG) };
    // Composite temperature is a little-endian 16-bit value in Kelvin per the NVMe spec.
    let kelvin = u16::from_le_bytes(log.Temperature);
    if kelvin == 0 {
        return None;
    }
    Some(f64::from(kelvin) - 273.15)
}

fn query_product_name(handle: HANDLE) -> Option<String> {
    let mut query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut buf = vec![0u8; 1024];
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &mut query as *mut _ as *mut _,
            std::mem::size_of_val(&query) as u32,
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 || returned < 28 {
        return None;
    }
    // STORAGE_DEVICE_DESCRIPTOR: ProductIdOffset at offset 16 (u32)
    let product_offset = u32::from_le_bytes(buf[16..20].try_into().ok()?) as usize;
    if product_offset == 0 || product_offset >= returned as usize {
        return None;
    }
    let slice = &buf[product_offset..returned as usize];
    let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    let s = String::from_utf8_lossy(&slice[..end]).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
