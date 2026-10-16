//! Table of lighting controllers we know about (by USB VID/PID).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownController {
    pub vid: u16,
    pub pid: u16,
    pub name: &'static str,
    /// What we can do with it today.
    pub status: &'static str,
}

pub const KNOWN_CONTROLLERS: &[KnownController] = &[KnownController {
    vid: 0x0DB0,
    pid: 0x0076,
    name: "MSI Mystic Light (X870 / B850 class)",
    status: "experimental: direct ARGB frames (report 0x51)",
}];

pub fn lookup_known(vid: u16, pid: u16) -> Option<&'static KnownController> {
    KNOWN_CONTROLLERS
        .iter()
        .find(|k| k.vid == vid && k.pid == pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_msi_mystic_light() {
        assert!(lookup_known(0x0DB0, 0x0076).is_some());
        assert!(lookup_known(0x0DB0, 0x0077).is_none());
    }
}
