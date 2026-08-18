//! Windows storage temperatures via `DeviceIoControl` - **no PowerShell**.
//!
//! Priority (live NVMe first - avoids a stuck/stale first TemperatureInfo slot):
//! 1. NVMe health-info log page (composite temp, Kelvin → °C)
//! 2. `StorageDeviceTemperatureProperty` - scan **all** `TemperatureInfo` entries
//! 3. `StorageAdapterTemperatureProperty` - same scan
//!
//! Opening `\\.\PhysicalDriveN` usually needs elevation.

#![cfg(windows)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, STORAGE_PROPERTY_QUERY,
    STORAGE_TEMPERATURE_DATA_DESCRIPTOR, STORAGE_TEMPERATURE_INFO,
    STORAGE_TEMPERATURE_VALUE_NOT_REPORTED, StorageAdapterTemperatureProperty,
    StorageDeviceProperty, StorageDeviceTemperatureProperty,
};

/// Probe `\\.\PhysicalDrive0` … for temperatures reported by the storage stack.
/// Returns `(id, display_name, temp_c, source_label)` for diagnostics via tracing.
pub fn probe_storage_temps() -> Vec<(String, String, f64)> {
    let mut rows = Vec::new();
    for index in 0..16u32 {
        let path = format!(r"\\.\PhysicalDrive{index}");
        let Some((name, temp, source)) = probe_one_drive(&path, index) else {
            continue;
        };
        if !(1.0..=100.0).contains(&temp) {
            // Reject 0 °C and absurd values (often thresholds / not reported)
            tracing::debug!(
                index,
                temp,
                source,
                "storage temp out of sensible range, skip"
            );
            continue;
        }
        tracing::debug!(index, name = %name, temp, source, "storage temp");
        rows.push((
            format!("host.ssd{index}"),
            format!("Storage ({name})"),
            temp,
        ));
    }
    rows
}

/// Public diagnostic: dump each path's reading for every openable drive (Admin).
pub fn diagnose_storage_temps() -> Vec<StorageDiag> {
    let mut out = Vec::new();
    for index in 0..16u32 {
        let path = format!(r"\\.\PhysicalDrive{index}");
        let Some(handle) = open_drive(&path) else {
            continue;
        };
        let name = query_product_name(handle).unwrap_or_else(|| format!("PhysicalDrive{index}"));
        let nvme = query_temperature_nvme(handle);
        let device = query_temperature_property(handle, StorageDeviceTemperatureProperty);
        let adapter = query_temperature_property(handle, StorageAdapterTemperatureProperty);
        let chosen = pick_best_temp(&[
            ("nvme_health", nvme),
            ("device_temp_prop", device),
            ("adapter_temp_prop", adapter),
        ]);
        unsafe {
            CloseHandle(handle);
        }
        out.push(StorageDiag {
            index,
            name,
            nvme_c: nvme,
            device_prop_c: device,
            adapter_prop_c: adapter,
            chosen_c: chosen.map(|(t, _)| t),
            chosen_source: chosen.map(|(_, s)| s.to_string()),
        });
    }
    out
}

#[derive(Debug, Clone)]
pub struct StorageDiag {
    pub index: u32,
    pub name: String,
    pub nvme_c: Option<f64>,
    pub device_prop_c: Option<f64>,
    pub adapter_prop_c: Option<f64>,
    pub chosen_c: Option<f64>,
    pub chosen_source: Option<String>,
}

fn probe_one_drive(path: &str, index: u32) -> Option<(String, f64, &'static str)> {
    let handle = open_drive(path)?;
    let result = (|| {
        let name = query_product_name(handle).unwrap_or_else(|| format!("PhysicalDrive{index}"));
        // Prefer NVMe health composite (updates under load) over property descriptors
        // that sometimes return a fixed first-slot / threshold-like value.
        let candidates = [
            ("nvme_health", query_temperature_nvme(handle)),
            (
                "device_temp_prop",
                query_temperature_property(handle, StorageDeviceTemperatureProperty),
            ),
            (
                "adapter_temp_prop",
                query_temperature_property(handle, StorageAdapterTemperatureProperty),
            ),
        ];
        let (temp, source) = pick_best_temp(&candidates)?;
        Some((name, temp, source))
    })();
    unsafe {
        CloseHandle(handle);
    }
    result
}

/// Prefer NVMe if present; else any valid property reading.
fn pick_best_temp(candidates: &[(&str, Option<f64>)]) -> Option<(f64, &'static str)> {
    // Stable preference order as listed
    for (src, t) in candidates {
        if let Some(v) = t
            && (1.0..=100.0).contains(v)
        {
            // SAFETY: src is static str from our array
            let label: &'static str = match *src {
                "nvme_health" => "nvme_health",
                "device_temp_prop" => "device_temp_prop",
                "adapter_temp_prop" => "adapter_temp_prop",
                _ => "unknown",
            };
            return Some((*v, label));
        }
    }
    None
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

fn query_temperature_property(handle: HANDLE, property_id: i32) -> Option<f64> {
    let mut query = STORAGE_PROPERTY_QUERY {
        PropertyId: property_id,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    // Room for descriptor header + several STORAGE_TEMPERATURE_INFO entries
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
    // Flexible array starts at TemperatureInfo - not `size_of(desc) - size_of(info)`,
    // which can be wrong if the trailing [1] is padded into the parent size.
    let info_offset = std::mem::offset_of!(STORAGE_TEMPERATURE_DATA_DESCRIPTOR, TemperatureInfo);
    let entry_size = std::mem::size_of::<STORAGE_TEMPERATURE_INFO>();
    if ok == 0 || (returned as usize) < info_offset + entry_size {
        return None;
    }
    // InfoCount is a u16 at a fixed offset; read unaligned-safe via byte copy.
    let count = {
        let off = std::mem::offset_of!(STORAGE_TEMPERATURE_DATA_DESCRIPTOR, InfoCount);
        if (returned as usize) < off + 2 {
            return None;
        }
        u16::from_le_bytes([buf[off], buf[off + 1]]) as usize
    };
    if count == 0 {
        return None;
    }
    let mut best: Option<f64> = None;
    for i in 0..count {
        let off = info_offset + i * entry_size;
        if off + entry_size > returned as usize {
            break;
        }
        // SAFETY: bounds checked; use read_unaligned (IOCTL buffer, no align guarantee).
        let info = unsafe {
            ptr::read_unaligned(buf.as_ptr().add(off) as *const STORAGE_TEMPERATURE_INFO)
        };
        let t = info.Temperature;
        if (t as u16) == (STORAGE_TEMPERATURE_VALUE_NOT_REPORTED as u16) {
            continue;
        }
        let c = f64::from(t);
        if !(1.0..=100.0).contains(&c) {
            continue;
        }
        // Prefer the highest reported sensor temp among valid readings (composite-like).
        best = Some(best.map(|b| b.max(c)).unwrap_or(c));
    }
    best
}

/// NVMe health-info log page: composite temperature in Kelvin.
fn query_temperature_nvme(handle: HANDLE) -> Option<f64> {
    use windows_sys::Win32::Storage::Nvme::NVME_HEALTH_INFO_LOG;
    use windows_sys::Win32::System::Ioctl::{
        NVMeDataTypeLogPage, ProtocolTypeNvme, STORAGE_PROTOCOL_DATA_DESCRIPTOR,
        STORAGE_PROTOCOL_SPECIFIC_DATA, StorageDeviceProtocolSpecificProperty,
    };

    const NVME_LOG_PAGE_HEALTH_INFO: u32 = 0x02;

    let protocol_data_len = std::mem::size_of::<STORAGE_PROTOCOL_SPECIFIC_DATA>();
    let health_len = std::mem::size_of::<NVME_HEALTH_INFO_LOG>();
    // CRITICAL: do NOT use `size_of::<STORAGE_PROPERTY_QUERY>() - 1`.
    // In Rust, that struct is padded to 12 bytes; size-1 = 11 → misaligned write
    // of STORAGE_PROTOCOL_SPECIFIC_DATA (align 4) and abort in debug builds.
    // AdditionalParameters starts at offset 8 (after PropertyId + QueryType).
    let query_offset = std::mem::offset_of!(STORAGE_PROPERTY_QUERY, AdditionalParameters);

    let mut in_buf = vec![0u8; query_offset + protocol_data_len + health_len];
    // Write PropertyId / QueryType at the start (buf is always well-aligned).
    unsafe {
        let q = in_buf.as_mut_ptr() as *mut STORAGE_PROPERTY_QUERY;
        (*q).PropertyId = StorageDeviceProtocolSpecificProperty;
        (*q).QueryType = PropertyStandardQuery;
    }
    let protocol = STORAGE_PROTOCOL_SPECIFIC_DATA {
        ProtocolType: ProtocolTypeNvme,
        DataType: NVMeDataTypeLogPage as u32,
        ProtocolDataRequestValue: NVME_LOG_PAGE_HEALTH_INFO,
        ProtocolDataRequestSubValue: 0,
        // Offset of protocol data relative to start of STORAGE_PROTOCOL_SPECIFIC_DATA
        ProtocolDataOffset: protocol_data_len as u32,
        ProtocolDataLength: health_len as u32,
        FixedProtocolReturnData: 0,
        ProtocolDataRequestSubValue2: 0,
        ProtocolDataRequestSubValue3: 0,
        ProtocolDataRequestSubValue4: 0,
    };
    // query_offset is 8 → 4-byte aligned relative to vec allocation.
    debug_assert_eq!(
        query_offset % std::mem::align_of::<STORAGE_PROTOCOL_SPECIFIC_DATA>(),
        0
    );
    unsafe {
        ptr::write(
            in_buf.as_mut_ptr().add(query_offset) as *mut STORAGE_PROTOCOL_SPECIFIC_DATA,
            protocol,
        );
    }

    let out_need =
        std::mem::size_of::<STORAGE_PROTOCOL_DATA_DESCRIPTOR>() + health_len + protocol_data_len;
    let mut out = vec![0u8; out_need.max(1024)];
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
    let desc: STORAGE_PROTOCOL_DATA_DESCRIPTOR =
        unsafe { ptr::read_unaligned(out.as_ptr() as *const STORAGE_PROTOCOL_DATA_DESCRIPTOR) };
    // ProtocolSpecificData sits at end of descriptor; log bytes follow at ProtocolDataOffset
    // relative to ProtocolSpecificData start.
    let protocol_data_start = (desc_header_len as usize).saturating_sub(protocol_data_len);
    let log_offset = protocol_data_start + desc.ProtocolSpecificData.ProtocolDataOffset as usize;
    let log_end = log_offset + health_len;
    if (returned as usize) < log_end {
        // Some drivers put data at a fixed offset after ProtocolSpecificData
        let alt = protocol_data_start + protocol_data_len;
        if (returned as usize) < alt + health_len {
            return None;
        }
        return kelvin_to_c_from_bytes(&out[alt..alt + health_len]);
    }
    if log_offset + health_len > out.len() {
        return None;
    }
    kelvin_to_c_from_bytes(&out[log_offset..log_offset + health_len])
}

fn kelvin_to_c_from_bytes(bytes: &[u8]) -> Option<f64> {
    use windows_sys::Win32::Storage::Nvme::NVME_HEALTH_INFO_LOG;
    if bytes.len() < std::mem::size_of::<NVME_HEALTH_INFO_LOG>() {
        return None;
    }
    let log: NVME_HEALTH_INFO_LOG =
        unsafe { ptr::read_unaligned(bytes.as_ptr() as *const NVME_HEALTH_INFO_LOG) };
    let kelvin = u16::from_le_bytes(log.Temperature);
    if !(200..=400).contains(&kelvin) {
        // Not a plausible absolute temperature in Kelvin
        return None;
    }
    let c = f64::from(kelvin) - 273.15;
    if (1.0..=100.0).contains(&c) {
        Some(c)
    } else {
        None
    }
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
    let product_offset = u32::from_le_bytes(buf[16..20].try_into().ok()?) as usize;
    if product_offset == 0 || product_offset >= returned as usize {
        return None;
    }
    let slice = &buf[product_offset..returned as usize];
    let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    let s = String::from_utf8_lossy(&slice[..end]).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}
