//! AMD 3D V-Cache presence.
//!
//! The mode attribute is `/sys/bus/platform/drivers/amd_x3d_vcache/<ACPI
//! id>/amd_x3d_mode`. The ACPI instance id differs between boards, so the
//! device is found by listing the driver directory, never named; the helper
//! writes the attribute the same way.

/// Whether this CPU exposes the V-Cache mode control.
#[must_use]
pub fn is_available() -> bool {
    crate::hardware::detect_vcache().is_some()
}
