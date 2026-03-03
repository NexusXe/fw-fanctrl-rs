use std::path::Path;

use crate::{Args, DEFAULT_CONFIG_PATH, USE_ONCE_PATH, fan_curve, fans, info, temp};

pub(super) fn run_daemon(
    profile: &fan_curve::FanProfile,
    args: &Args,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("[ERROR]: Error setting Ctrl-C handler");

    info!(
        "Starting daemon with profile \"{}\". Using {:}ms sleep.",
        args.profile, args.sleep_millis
    );

    while running.load(Ordering::SeqCst) {
        let max_temp = temp::get_max_temp()?;
        let speed = profile.get_fan_speed(max_temp);
        fans::set_duty(speed)?;
        if args.verbose {
            info!("{:}°C: {speed:3}%", max_temp.to_celsius().0);
        }
        std::thread::sleep(std::time::Duration::from_millis(args.sleep_millis));
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
