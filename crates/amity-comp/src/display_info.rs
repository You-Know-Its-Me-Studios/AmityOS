//! Monitor make and model from a DRM connector's EDID.
//!
//! smithay-drm-extras 0.1 binds libdisplay-info 0.2, but Ubuntu 26.04 ships
//! libdisplay-info 0.3, so the lookup is done here with the 0.3 bindings.

use libdisplay_info::info::Info;
use smithay::reexports::drm::control::{connector, Device as ControlDevice};

/// Reads and parses the connector's `EDID` property.
pub fn for_connector(device: &impl ControlDevice, connector: connector::Handle) -> Option<Info> {
    let props = device.get_properties(connector).ok()?;
    let (info, value) = props
        .into_iter()
        .filter_map(|(handle, value)| Some((device.get_property(handle).ok()?, value)))
        .find(|(info, _)| info.name().to_str() == Ok("EDID"))?;
    let blob = info.value_type().convert_value(value).as_blob()?;
    let data = device.get_property_blob(blob).ok()?;
    Info::parse_edid(&data).ok()
}
