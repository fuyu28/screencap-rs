//! Port of src/monitor_enum.cpp.

use crate::types::MonitorInfo;

/// EnumDisplayMonitors; index is enumeration order.
pub fn enumerate_monitors() -> Vec<MonitorInfo> {
    todo!("port EnumerateMonitors")
}

/// token is "primary" or a monitor index in decimal.
pub fn find_monitor_by_token(_monitors: &[MonitorInfo], _token: &str) -> Option<MonitorInfo> {
    todo!("port FindMonitorByToken")
}
