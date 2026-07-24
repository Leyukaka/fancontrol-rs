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
    if ok == 0 || returned < min {
        return None;
    }
    let desc = unsafe { &*(buf.as_ptr() as *const STORAGE_TEMPERATURE_DATA_DESCRIPTOR) };
    if desc.InfoCount == 0 {
        return None;
    }
    let t = desc.TemperatureInfo[0].Temperature;
    // 0x8000 = not reported (STORAGE_TEMPERATURE_VALUE_NOT_REPORTED)
    if (t as u16) == (STORAGE_TEMPERATURE_VALUE_NOT_REPORTED as u16) {
        return None;
    }
    Some(f64::from(t))
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
