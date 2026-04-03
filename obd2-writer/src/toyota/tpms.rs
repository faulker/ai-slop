use crate::error::Result;
use crate::protocol::uds;
use crate::transport::elm327::Elm327;

/// Register TPMS sensor IDs using UDS RoutineControl (0x31).
/// This is vehicle-specific and may require the correct routine ID for Toyota.
#[allow(dead_code)]
pub async fn register_tpms_sensors(elm: &mut Elm327, sensor_ids: &[u32]) -> Result<()> {
    // Set target to BCM/TPMS ECU (address varies by vehicle)
    elm.set_header("750").await?;

    // Enter Extended Diagnostic Session
    let session_req = uds::diagnostic_session_control(uds::SESSION_EXTENDED);
    uds::send_uds(elm, &session_req).await?;

    println!("Registering {} TPMS sensor(s)...", sensor_ids.len());

    for (i, &sensor_id) in sensor_ids.iter().enumerate() {
        let id_bytes = sensor_id.to_be_bytes();
        // Routine ID for TPMS registration is vehicle-specific
        // Toyota commonly uses 0xFF00 range for TPMS routines
        let routine_req = uds::routine_control(0x01, 0xFF01, &id_bytes);
        let resp = uds::send_uds(elm, &routine_req).await?;
        println!("Sensor {} (ID: 0x{:08X}): {}", i + 1, sensor_id, uds::hex_string(&resp));
    }

    // Return to default session
    let default_req = uds::diagnostic_session_control(uds::SESSION_DEFAULT);
    let _ = uds::send_uds(elm, &default_req).await;

    println!("TPMS registration complete.");
    Ok(())
}
