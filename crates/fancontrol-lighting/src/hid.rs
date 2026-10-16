//! Minimal Windows HID access (CfgMgr32 + hid.dll). No driver, no elevation needed
//! for vendor-defined devices such as lighting controllers.

#![cfg(windows)]

use crate::{LightingError, Result};
use std::ptr;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW,
    CM_Get_Device_Interface_ListW, CR_BUFFER_SMALL, CR_SUCCESS,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_STATUS_SUCCESS, HidD_FreePreparsedData, HidD_GetAttributes,
    HidD_GetFeature, HidD_GetHidGuid, HidD_GetManufacturerString, HidD_GetPreparsedData,
    HidD_GetProductString, HidD_GetSerialNumberString, HidD_SetFeature, HidP_GetCaps,
    PHIDP_PREPARSED_DATA,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::core::GUID;

/// One HID interface (a USB device can expose several).
#[derive(Debug, Clone, Default)]
pub struct HidDeviceInfo {
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    pub version: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    pub usage_page: u16,
    pub usage: u16,
    pub input_len: u16,
    pub output_len: u16,
    pub feature_len: u16,
}

/// Owned handle, closed on drop.
pub struct HidDevice {
    handle: HANDLE,
}

impl Drop for HidDevice {
    fn drop(&mut self) {
        // SAFETY: handle came from a successful CreateFileW and is closed once.
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

impl HidDevice {
    /// Open for feature report get/set.
    pub fn open(path: &str) -> Result<Self> {
        open_path(path, GENERIC_READ | GENERIC_WRITE)
            .map(|handle| Self { handle })
            .ok_or_else(|| last_error(&format!("open {path}")))
    }

    /// Read feature report `report_id` into a buffer of `len` bytes (report id included).
    pub fn get_feature(&self, report_id: u8, len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len.max(1)];
        buf[0] = report_id;
        let len = buf.len() as u32;
        // SAFETY: buffer is valid for `buf.len()` bytes for the duration of the call.
        let ok = unsafe { HidD_GetFeature(self.handle, buf.as_mut_ptr().cast(), len) };
        if ok {
            Ok(buf)
        } else {
            Err(last_error(&format!("get feature 0x{report_id:02X}")))
        }
    }

    /// Send a feature report (`report[0]` is the report id).
    pub fn set_feature(&self, report: &[u8]) -> Result<()> {
        if report.is_empty() {
            return Err(LightingError::Hid("empty feature report".into()));
        }
        let len = report.len() as u32;
        // SAFETY: buffer is valid for `report.len()` bytes for the duration of the call.
        let ok = unsafe { HidD_SetFeature(self.handle, report.as_ptr().cast(), len) };
        if ok {
            Ok(())
        } else {
            Err(last_error(&format!("set feature 0x{:02X}", report[0])))
        }
    }
}

/// List every present HID interface. Devices that refuse to open are skipped.
pub fn enumerate() -> Vec<HidDeviceInfo> {
    interface_paths()
        .into_iter()
        .filter_map(|path| describe(&path))
        .collect()
}

fn hid_guid() -> GUID {
    let mut guid = GUID::from_u128(0);
    // SAFETY: writes one GUID into a valid out pointer.
    unsafe { HidD_GetHidGuid(&mut guid) };
    guid
}

fn interface_paths() -> Vec<String> {
    let guid = hid_guid();
    // The list can grow between the size query and the fetch (hotplug): retry a few times.
    for _ in 0..4 {
        let mut len = 0u32;
        // SAFETY: valid out pointer and GUID; null device id means "all".
        let cr = unsafe {
            CM_Get_Device_Interface_List_SizeW(
                &mut len,
                &guid,
                ptr::null(),
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            )
        };
        if cr != CR_SUCCESS || len == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        // SAFETY: buffer holds `len` UTF-16 units as requested.
        let cr = unsafe {
            CM_Get_Device_Interface_ListW(
                &guid,
                ptr::null(),
                buf.as_mut_ptr(),
                len,
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            )
        };
        if cr == CR_BUFFER_SMALL {
            continue;
        }
        if cr != CR_SUCCESS {
            return Vec::new();
        }
        // Multi-sz: strings separated by NUL, terminated by an empty string.
        return buf
            .split(|&c| c == 0)
            .filter(|s| !s.is_empty())
            .map(String::from_utf16_lossy)
            .collect();
    }
    Vec::new()
}

fn describe(path: &str) -> Option<HidDeviceInfo> {
    // Access 0: attributes and caps work even on devices held exclusively (keyboards, mice).
    let handle = open_path(path, 0)?;
    let mut info = HidDeviceInfo {
        path: path.to_string(),
        ..Default::default()
    };
    let mut attrs = HIDD_ATTRIBUTES {
        Size: size_of::<HIDD_ATTRIBUTES>() as u32,
        VendorID: 0,
        ProductID: 0,
        VersionNumber: 0,
    };
    // SAFETY: valid handle and out struct with Size set.
    if unsafe { HidD_GetAttributes(handle, &mut attrs) } {
        info.vid = attrs.VendorID;
        info.pid = attrs.ProductID;
        info.version = attrs.VersionNumber;
    }
    info.manufacturer = read_string(handle, HidD_GetManufacturerString);
    info.product = read_string(handle, HidD_GetProductString);
    info.serial = read_string(handle, HidD_GetSerialNumberString);

    let mut preparsed: PHIDP_PREPARSED_DATA = 0;
    // SAFETY: valid handle and out pointer; freed below on success.
    if unsafe { HidD_GetPreparsedData(handle, &mut preparsed) } {
        let mut caps = HIDP_CAPS::default();
        // SAFETY: preparsed data is valid until HidD_FreePreparsedData.
        if unsafe { HidP_GetCaps(preparsed, &mut caps) } == HIDP_STATUS_SUCCESS {
            info.usage_page = caps.UsagePage;
            info.usage = caps.Usage;
            info.input_len = caps.InputReportByteLength;
            info.output_len = caps.OutputReportByteLength;
            info.feature_len = caps.FeatureReportByteLength;
        }
        // SAFETY: frees the preparsed data obtained above, once.
        unsafe { HidD_FreePreparsedData(preparsed) };
    }
    // SAFETY: handle from open_path, closed once.
    unsafe { CloseHandle(handle) };
    Some(info)
}

type StringFn = unsafe extern "system" fn(HANDLE, *mut core::ffi::c_void, u32) -> bool;

fn read_string(handle: HANDLE, f: StringFn) -> Option<String> {
    // USB string descriptors are at most 126 UTF-16 units.
    let mut buf = [0u16; 128];
    // SAFETY: buffer valid for its byte size.
    let ok = unsafe { f(handle, buf.as_mut_ptr().cast(), size_of_val(&buf) as u32) };
    if !ok {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let s = String::from_utf16_lossy(&buf[..end]).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn open_path(path: &str, access: u32) -> Option<HANDLE> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: NUL-terminated wide path; no security attributes, no template.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    (handle != INVALID_HANDLE_VALUE).then_some(handle)
}

fn last_error(what: &str) -> LightingError {
    // SAFETY: reads the calling thread's last-error value.
    let code = unsafe { GetLastError() };
    LightingError::Hid(format!("{what} failed (Win32 error {code})"))
}
