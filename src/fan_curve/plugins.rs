use std::{
    cell::UnsafeCell, collections::HashMap, ffi::{CStr, CString, c_char}, num::NonZeroU16, sync::{LazyLock, Mutex}, thread, time::{Duration, Instant}
};

use crate::{
    infov,
    temp::{
        EC_TEMP_SENSOR_ENTRIES, EcResponseTempSensorGetInfo, NUM_TEMP_SENSORS, TempSensorVector,
        UnvalidatedEcTemp, ValidEcTemp, get_temperatures_v, probe_sensor,
    },
    warn,
};

pub(crate) static ALL_SENSORS: LazyLock<[EcResponseTempSensorGetInfo; EC_TEMP_SENSOR_ENTRIES]> =
    LazyLock::new(|| {
        let mut sensors = [EcResponseTempSensorGetInfo::default(); EC_TEMP_SENSOR_ENTRIES];

        let limit = NUM_TEMP_SENSORS.min(EC_TEMP_SENSOR_ENTRIES as u8);

        for id in 0..limit {
            if let Ok(info) = probe_sensor(id) {
                sensors[id as usize] = info;
            } else {
                warn!("Failed to probe sensor {id:}, despite EC reporting {limit:} sensors.");
            }
        }

        sensors
    });

#[repr(C)]
pub enum PluginGetStatus {
    /// The buffer was large enough and the data was successfully copied.
    Success = 0,
    /// The key does not exist or arguments were null.
    NotFound = 1,
    /// The key exists, but the provided buffer was null or too small.
    /// The required size has been written to the length out-parameter.
    BufferTooSmall = 2,
    /// Some other error occurred.
    SomeOtherError = 3,
}

/// stupid bullshit that allows me to access this data if a thread locks it
/// and then panics
pub struct ForceableLock<T> {
    lock: Mutex<()>,         // Just handles the blocking/state
    data: UnsafeCell<T>,     // Holds the actual data, explicitly allows aliasing
}

// YES I KNOW WHAT I AM SIGNING UP FOR JUST LET ME DO IT
unsafe impl<T: Send> Sync for ForceableLock<T> {}

impl<T> ForceableLock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            lock: Mutex::new(()),
            data: UnsafeCell::new(data),
        }
    }
}

/// Plugin state. Static because I want it to be lol
static PLUGIN_STATE: LazyLock<ForceableLock<HashMap<CString, Box<[u8]>>>> =
    LazyLock::new(|| ForceableLock::new(HashMap::new()));

/// Dumps the plugin state to the console.
///
/// This function is used for debugging plugins.
///
/// SAFETY: It isn't lol
#[allow(clippy::collection_is_never_read)] // intentional
#[allow(clippy::significant_drop_tightening)] // also intentional
pub(crate) unsafe fn dump_plugin_state() {
    let mut _guard_store = None;
    let start = Instant::now();
    
    let data: &HashMap<CString, Box<[u8]>> = loop {
        match PLUGIN_STATE.lock.try_lock() {
            Ok(guard) => {
                _guard_store = Some(guard);
                // Safe-ish: We hold the lock, so we can read the data.
                break unsafe { &*PLUGIN_STATE.data.get() };
            }
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                _guard_store = Some(poisoned.into_inner());
                break unsafe { &*PLUGIN_STATE.data.get() };
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                if start.elapsed() >= Duration::from_secs(1) {
                    warn!(
                        "PLUGIN_STATE lock timed out after 1s; forcing dirty read via UnsafeCell."
                    );
                    // The "Proper" UB: We don't have the lock, but we are legally 
                    // bypassing the compiler's reference rules to perform a dirty read.
                    break unsafe { &*PLUGIN_STATE.data.get() };
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    };

    for (key, value) in data.iter() {
        let key_str = key.to_string_lossy();
        // Assuming infov! is a macro you have defined
        infov!("\t{}: {}", key_str, value.len());
    }
}

/// Helper function for plugins to set a key-value pair in the plugin state.
///
/// If this function panics, the sun will explode.
pub(crate) extern "C" fn plugin_set(key: *const c_char, data: *const u8, len: usize) -> bool {
    if key.is_null() || (data.is_null() && len > 0) {
        return false;
    }

    let c_str = unsafe { CStr::from_ptr(key) };
    let c_string = CString::from(c_str);

    let data_slice = if len == 0 {
        &[] 
    } else {
        unsafe { std::slice::from_raw_parts(data, len) }
    };

    let owned_data = data_slice.to_vec().into_boxed_slice();

    // 1. Acquire the permission slip (lock the Mutex<()>)
    let _guard = if let Ok(l) = PLUGIN_STATE.lock.lock() {
        l
    } else {
        return false; // Mutex is poisoned, abort.
    };

    // 2. Access the data
    // SAFETY: We hold `_guard`, ensuring exclusive access to the UnsafeCell.
    let map = unsafe { &mut *PLUGIN_STATE.data.get() };
    
    map.insert(c_string, owned_data);

    true 
    // `_guard` drops here, unlocking the Mutex automatically.
}

/// Helper function for plugins to get a key-value pair from the plugin state.
///
/// Tries its darndest to keep the sun from exploding.
pub(crate) extern "C" fn plugin_get(
    key: *const c_char,
    buffer: *mut u8,
    buffer_len: *mut usize,
) -> PluginGetStatus {
    let result = plugin_get_fallible(key, buffer, buffer_len);
    result.unwrap_or(PluginGetStatus::SomeOtherError)
}

/// Helper function for plugins to get a key-value pair from the plugin state.
///
/// Returns `None` if the key does not exist, or if the buffer is null or too small.
#[inline]
fn plugin_get_fallible(
    key: *const c_char,
    buffer: *mut u8,
    buffer_len: *mut usize,
) -> Option<PluginGetStatus> {
    if key.is_null() || buffer_len.is_null() {
        return Some(PluginGetStatus::NotFound);
    }

    let c_str = unsafe { CStr::from_ptr(key) };

    // 1. Acquire the permission slip. 
    // If it fails (poisoned), the `?` returns None.
    let _guard = PLUGIN_STATE.lock.lock().ok()?;

    // 2. Access the data immutably.
    // SAFETY: We hold `_guard`, ensuring no other thread is mutating the UnsafeCell.
    let map = unsafe { &*PLUGIN_STATE.data.get() };

    if let Some(val) = map.get(c_str) {
        let required_len = val.len();
        let provided_capacity = unsafe { *buffer_len };

        unsafe { *buffer_len = required_len };

        if buffer.is_null() || provided_capacity < required_len {
            return Some(PluginGetStatus::BufferTooSmall);
        }

        unsafe {
            std::ptr::copy_nonoverlapping(val.as_ptr(), buffer, required_len);
        }

        return Some(PluginGetStatus::Success); 
    }

    Some(PluginGetStatus::NotFound)
    // `_guard` drops here, unlocking the Mutex automatically.
}

#[repr(C)]
pub struct PluginStateMethods {
    pub set: extern "C" fn(*const c_char, *const u8, usize) -> bool,
    pub get: extern "C" fn(*const c_char, *mut u8, *mut usize) -> PluginGetStatus,
}

static STATE: PluginStateMethods = PluginStateMethods {
    set: plugin_set,
    get: plugin_get,
};

#[repr(C)]
pub(crate) struct AncillaryInfo {
    highest_temp: ValidEcTemp,
    num_sensors: u8,
    lut_speed: u8,
}

#[repr(C)]
pub(crate) struct PluginCallData {
    sensors: *const EcResponseTempSensorGetInfo,
    temps: *const TempSensorVector,
    state: *const PluginStateMethods,
    ancillary: AncillaryInfo,
}

#[repr(C)]
#[allow(dead_code)]
pub(crate) enum DecisionValue<T> {
    /// Set the fan speed directly to the given value.
    SetSpeed(u8),
    /// Get the fan speed from the current curve, using the given temperature.
    GetSpeedFromCurve(T),
}

#[repr(C)]
pub(crate) struct Decision<T> {
    /// The decision from the plugin; either set the fan speed directly, or get it from the current curve.
    pub(crate) value: DecisionValue<T>,
    /// Run again in `run_again_in` milliseconds. If `None`, run again after the configured sleep duration.
    pub(crate) run_again_in: Option<NonZeroU16>,
}

pub(crate) type PluginDecision = Decision<UnvalidatedEcTemp>;
pub(crate) type ValidatedDecision = Decision<ValidEcTemp>;

pub(crate) type PluginFn = extern "C" fn(*const PluginCallData) -> PluginDecision;

/// Unfinished plugin interface. Calls a shared object file
pub(crate) fn call_plugin(
    plugin_fn: PluginFn,
    highest_temp: ValidEcTemp,
    lut_speed: u8,
) -> Result<ValidatedDecision, Box<dyn std::error::Error>> {
    let readings = get_temperatures_v()?;

    let ancillary_info = AncillaryInfo {
        highest_temp,
        num_sensors: *NUM_TEMP_SENSORS,
        lut_speed,
    };

    let readings_ptr = &raw const readings;
    let sensors_ptr = ALL_SENSORS.as_ptr();

    let call_data = PluginCallData {
        sensors: sensors_ptr,
        temps: readings_ptr,
        state: &STATE,
        ancillary: ancillary_info,
    };

    let plugin_output = plugin_fn(&call_data);

    // validate plugin_output
    let validated_output = match plugin_output.value {
        DecisionValue::SetSpeed(speed) => {
            if speed > 100 {
                return Err("Plugin set speed above 100%".into());
            }
            ValidatedDecision {
                value: DecisionValue::SetSpeed(speed),
                run_again_in: plugin_output.run_again_in,
            }
        }
        DecisionValue::GetSpeedFromCurve(temp) => {
            let valid_temp = temp.validate().map_err(|e| {
                format!("Plugin get speed from curve with invalid temperature: {e}")
            })?;
            ValidatedDecision {
                value: DecisionValue::GetSpeedFromCurve(valid_temp),
                run_again_in: plugin_output.run_again_in,
            }
        }
    };

    Ok(validated_output)
}
