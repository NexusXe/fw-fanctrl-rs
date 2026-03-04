#[cfg(feature = "plugin")]
use nix::time::{ClockId, clock_gettime};

use crate::{DEFAULT_CONFIG_PATH, USE_ONCE_PATH, fan_curve, fans, info};
use std::{
    num::NonZeroU64,
    path::{Path, PathBuf},
};

#[cfg(feature = "plugin")]
use crate::{
    infov, warn,
    fan_curve::plugins::{PluginFn, call_plugin},
};

use crate::{temp, verbose};

#[cfg(feature = "plugin")]
use std::sync::OnceLock;

// library needs to stay alive for the duration of the daemon, so we use a static OnceLock
// idk if this is the best way to do it
#[cfg(feature = "plugin")]
static PLUGIN_LIB: OnceLock<libloading::Library> = OnceLock::new();

#[cfg(feature = "plugin")]
fn init_plugin(plugin: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let lib = unsafe { libloading::Library::new(plugin) }?;
    PLUGIN_LIB
        .set(lib)
        .map_err(|_| "Plugin already initialized".into())
}

#[cfg(feature = "plugin")]
static PLUGIN_FN: OnceLock<PluginFn> = OnceLock::new();

#[cfg(feature = "plugin")]
fn init_plugin_fn(plugin: &Path) -> Result<(), Box<dyn std::error::Error>> {
    init_plugin(plugin)?;
    let lib = PLUGIN_LIB.get().ok_or("Plugin not initialized")?;
    let func = unsafe {
        lib.get(b"get_decision")
            .map(|f| *f)
            .expect("[ERROR]: Could not find C function get_decision in plugin")
    };
    PLUGIN_FN
        .set(func)
        .map_err(|_| "Plugin function already initialized".into())
}

pub(super) fn run_daemon(
    profile: &fan_curve::FanProfile,
    sleep_millis: NonZeroU64,
    _plugin: Option<&PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let running = Arc::new(AtomicBool::new(true));
    #[allow(clippy::redundant_clone)] // used by plugins
    let r = running.clone();
    // set handler so auto fan control can be re-enabled when daemon stops
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("[ERROR]: Error setting Ctrl-C handler");

    info!(
        "Starting daemon with profile \"{}\". Using {:}ms sleep.",
        profile.name, sleep_millis
    );

    #[cfg(feature = "plugin")]
    if let Some(plugin) = _plugin {
        init_plugin_fn(plugin)?;
    }

    #[cfg(feature = "plugin")]
    let mut bad_plugin: bool = false;

    #[cfg(feature = "plugin")]
    let mut last_poll = clock_gettime(ClockId::CLOCK_MONOTONIC_COARSE)?;

    while running.load(Ordering::SeqCst) {
        #[cfg(feature = "plugin")]
        let mut sleep_millis = sleep_millis;
        let max_temp = temp::get_max_temp()?;
        let lut_speed = profile.get_fan_speed(max_temp);
        let speed = {
            #[cfg(feature = "plugin")]
            {
                if bad_plugin || _plugin.is_none() {
                    lut_speed
                } else {
                    match call_plugin(
                        *PLUGIN_FN.get().ok_or("Plugin not initialized")?,
                        max_temp,
                        lut_speed,
                        last_poll,
                    ) {
                        Ok(decision) => {
                            if let Some(x) = decision.run_again_in {
                                sleep_millis = x.into();
                            }

                            match decision.value {
                                fan_curve::plugins::DecisionValue::SetSpeed(speed) => {
                                    infov!("Plugin wants to directly set speed to {speed}%");
                                    if speed > 100 {
                                        warn!(
                                            "Plugin set speed above 100%. Assuming plugin errored, and using LUT speed from now on."
                                        );
                                        bad_plugin = true;
                                        lut_speed
                                    } else {
                                        speed
                                    }
                                }
                                fan_curve::plugins::DecisionValue::GetSpeedFromCurve(temp) => {
                                    infov!(
                                        "Plugin wants to use curve speed for {:}°C",
                                        temp.to_celsius().0
                                    );
                                    profile.get_fan_speed(temp)
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Plugin error: {e}\n\tIgnoring plugin and using LUT speed from now on."
                            );
                            unsafe { fan_curve::plugins::dump_plugin_state() };
                            bad_plugin = true;
                            lut_speed
                        }
                    }
                }
            }

            #[cfg(not(feature = "plugin"))]
            {
                lut_speed
            }
        };

        fans::set_duty(speed)?;
        if verbose() {
            info!("{:}°C: {speed:3}%", max_temp.to_celsius().0);
        }

        #[cfg(feature = "plugin")]
        {   
            last_poll = clock_gettime(ClockId::CLOCK_MONOTONIC_COARSE)?;
        }
        std::thread::sleep(std::time::Duration::from_millis(sleep_millis.get()));
    }

    // Cleanup
    info!("\nShutting down...");
    fans::set_auto()?;
    info!("Set auto fan control.");
    Ok(())
}

pub(super) fn restart_daemon<const NEW_DEFAULT: bool>(
    new_curve: &str,
    profiles: &[fan_curve::FanProfile],
) -> Result<(), Box<dyn std::error::Error>> {
    use std::env;
    use std::process::Command;

    let uid = env::var("SUDO_UID").unwrap_or_else(|_| String::from("1000"));
    let service_name = format!("fw-fanctrl@{uid}.service");

    info!("Applying \"{new_curve}\" curve and restarting {service_name}...");

    // ensure the new curve exists in either the built-in profiles or external curves
    if fan_curve::get_profile_by_name(new_curve, profiles).is_none() {
        return Err(format!("[ERROR]: Could not find curve \"{new_curve}\".").into());
    }

    if NEW_DEFAULT {
        // open the default config file and replace the line that specifies the default curve
        let config = std::fs::read_to_string(DEFAULT_CONFIG_PATH)?;
        let config = config.replace(
            "default_curve = \"default\"",
            &format!("default_curve = \"{new_curve}\" # Set by fw-fanctrl-rs --use-default"),
        );
        match std::fs::write(DEFAULT_CONFIG_PATH, config) {
            Ok(()) => info!("Set \"{new_curve}\" as the new default curve."),
            Err(e) => {
                eprintln!("[ERROR]: Failed to set \"{new_curve}\" as the new default curve.");
                match e.kind() {
                    std::io::ErrorKind::PermissionDenied => {
                        return Err("[ERROR]: Permission denied. Are you running as root?".into());
                    }
                    _ => return Err(e.to_string().into()),
                }
            }
        }
    } else {
        // write the curve
        match std::fs::write(Path::new(USE_ONCE_PATH), new_curve) {
            Ok(()) => info!("Set \"{new_curve}\" as the curve to use once."),
            Err(e) => {
                eprintln!("[ERROR]: Failed to set \"{new_curve}\" as the curve to use once.");
                match e.kind() {
                    std::io::ErrorKind::PermissionDenied => {
                        return Err("[ERROR]: Permission denied. Are you running as root?".into());
                    }
                    _ => return Err(e.to_string().into()),
                }
            }
        }
    }

    let status = Command::new("systemctl")
        .arg("reload")
        .arg(&service_name)
        .status()?;

    if status.success() {
        info!("Daemon successfully reloaded.");
        Ok(())
    } else {
        Err(format!("[ERROR]: Failed to reload daemon. It may not be running. Try checking: systemctl status {service_name}").into())
    }
}
