#![feature(default_field_values)]
#![feature(generic_const_exprs)]
#![feature(const_cmp)]
#![feature(const_trait_impl)]
#![feature(const_convert)]
#![feature(const_default)]
#![feature(const_try)]
#![allow(incomplete_features)]
#![warn(clippy::pedantic)]
#![allow(clippy::cast_possible_truncation)]

pub(crate) mod common;
mod fan_curve;
mod fans;
mod temp;

use clap::Parser;
use std::{path::Path, sync::OnceLock};

use serde::Deserialize;
use std::fs;

#[derive(Deserialize)]
struct Config {
    default_curve: String,
    poll_interval_ms: u64,
}

const DEFAULT_CONFIG_PATH: &str = "/etc/fw-fanctrl-rs/config.toml";
const USE_ONCE_PATH: &str = "/etc/fw-fanctrl-rs/.use-once.tmp";

fn load_config(path: &str) -> Result<Config, Box<dyn std::error::Error>> {
    let config_str = fs::read_to_string(path)?;
    toml::from_str(&config_str).map_err(Into::into)
}

static QUIET: OnceLock<bool> = OnceLock::new();
static VERBOSE: OnceLock<bool> = OnceLock::new();

/// Returns `true` if the `--quiet` flag was passed.
fn quiet() -> bool {
    *QUIET.get().unwrap_or(&false)
}

/// Returns `true` if the `--verbose` flag was passed.
fn verbose() -> bool {
    *VERBOSE.get().unwrap_or(&false)
}

/// Helper for printing info messages when verbose
#[macro_export]
macro_rules! infov {
    ($($arg:tt)*) => {
        if $crate::verbose() {
            println!("[INFO(V)]: {}", format_args!($($arg)*));
        }
    };
}

/// Helper for printing info messages when not quiet
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        if !$crate::quiet() {
            println!("[INFO]: {}", format_args!($($arg)*));
        }
    };
}

/// Helper for printing warning messages when not quiet
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        if !$crate::quiet() {
            eprintln!("[WARN]: {}", format_args!($($arg)*));
        }
    };
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
#[allow(clippy::struct_excessive_bools)]
struct Args {
    /// List temperatures
    #[arg(short = 't', long)]
    temp: bool,

    /// Set fan speeds (0-100 or 'auto'). If passed without a value, defaults to 'auto'.
    #[arg(short = 'f', long, value_name = "SPEED", num_args = 0..=1, default_missing_value = "auto")]
    fan: Option<String>,

    /// Run as daemon
    #[arg(short = 'd', long, conflicts_with = "once")]
    daemon: bool,

    /// Sleep duration in milliseconds between checks
    #[arg(short = 's', long, default_value = "1000")]
    sleep_millis: u64,

    /// Check temps and set fans to match curve once
    #[arg(short = 'o', long)]
    once: bool,

    /// Print fan curve in CSV format
    #[arg(long)]
    curve: bool,

    /// Fan curve profile to use
    #[arg(short = 'p', long, default_value = "default-or-use-once")]
    profile: String,

    /// Generate shell completions
    #[arg(long, value_enum)]
    print_completions: Option<clap_complete::Shell>,

    /// Print total LUT size
    #[arg(long)]
    total_lut_size: bool,

    /// Verbose output
    #[arg(short = 'v', long)]
    verbose: bool,

    /// Quiet output
    #[arg(short = 'q', long, conflicts_with = "verbose")]
    quiet: bool,

    /// List external curves
    #[arg(
        long,
        conflicts_with = "once",
        conflicts_with = "daemon",
        conflicts_with = "fan",
        conflicts_with = "temp",
        conflicts_with = "curve",
        conflicts_with = "total_lut_size"
    )]
    list_external_curves: bool,

    /// Custom config path
    #[arg(short = 'c', long, default_value = DEFAULT_CONFIG_PATH)]
    config: String,

    /// Restart the daemon using a custom curve
    #[arg(
        short = 'u',
        long,
        conflicts_with = "once",
        conflicts_with = "fan",
        conflicts_with = "temp",
        conflicts_with = "curve",
        conflicts_with = "total_lut_size",
        conflicts_with = "list_external_curves"
    )]
    r#use: Option<String>,

    /// Restart the daemon using a custom curve, also setting it as the new default
    #[arg(
        short = 'U',
        long,
        conflicts_with = "once",
        conflicts_with = "fan",
        conflicts_with = "temp",
        conflicts_with = "curve",
        conflicts_with = "total_lut_size",
        conflicts_with = "list_external_curves",
        conflicts_with = "use"
    )]
    r#use_default: Option<String>,
}

#[allow(clippy::too_many_lines)] // too bad!
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use clap::CommandFactory;
    let mut args = Args::parse();
    if args.quiet {
        QUIET.set(true).unwrap();
    } else if args.verbose {
        // mutually exclusive with quiet
        VERBOSE.set(true).unwrap();
    }

    let mut config_default = None;

    if let Some(profile) = args.r#use {
        restart_daemon::<false>(&profile)?;
        return Ok(());
    } else if let Some(profile) = args.use_default {
        restart_daemon::<true>(&profile)?;
        return Ok(());
    }

    match load_config(&args.config) {
        Ok(config) => {
            infov!("Loaded config from {}", args.config);
            config_default = Some(config.default_curve);
            infov!(
                "    With default profile: {}",
                config_default.as_ref().unwrap()
            );
            args.sleep_millis = config.poll_interval_ms;
            infov!("    With poll interval: {}ms", args.sleep_millis);
        }
        Err(e) => {
            warn!("Failed to load config: {e}");
        }
    }

    let config_default = config_default.unwrap_or_else(|| "default".to_string());

    if args.profile == "default-or-use-once" && args.r#use.is_none() && args.use_default.is_none() {
        let use_once_path = Path::new(USE_ONCE_PATH);
        if use_once_path.exists() {
            infov!("Use-once file found, using profile from file.");
            args.profile = std::fs::read_to_string(use_once_path)?;
            // remove the use-once file
            std::fs::remove_file(use_once_path)?;
        } else {
            infov!("No use-once file found, using default profile.");
            args.profile = config_default;
        }
    } else {
        infov!("Using profile from command line: {}", args.profile);
    }

    if let Some(shell) = args.print_completions {
        let mut cmd = Args::command();
        clap_complete::generate(shell, &mut cmd, "fw-fanctrl-rs", &mut std::io::stdout());
        return Ok(());
    }

    let external_curves = fan_curve::curve_parsing::get_all_external_curves();

    let profile = fan_curve::get_profile_by_name(&args.profile, Some(&external_curves))
        .unwrap_or_else(|| {
            warn!("Profile '{}' not found, using default.", args.profile);
            fan_curve::get_profile_by_name("default", None).unwrap()
        });

    if args.temp {
        print_temps()?;
    } else if let Some(val) = args.fan {
        if val == "auto" {
            fans::set_auto()?;
            info!("Set auto fan control.");
        } else {
            let duty: u8 = val.parse::<u8>()?.clamp(0, 100);
            fans::set_duty(duty)?;
            info!("Set to {duty:}");
        }
    } else if args.once {
        // check temps and set fans to match curve
        let max_temp = temp::get_max_temp()?;
        let speed = profile.get_fan_speed(max_temp);
        fans::set_duty(speed)?;
        println!("[OUT]: {:}°C: {speed:3}%", max_temp.to_celsius().0);
    } else if args.daemon {
        run_daemon(profile, &args)?;
    } else if args.curve {
        println!("[OUT]: {profile}");
        // don't prefix with [OUT] for the CSV
        println!("Temperature (°C),PWM");
        for temp in 0..=u8::MAX - 4 {
            let temp = temp::ValidEcTemp(temp);
            println!("{:},{:}", temp.to_celsius().0, profile.get_fan_speed(temp));
        }
    } else if args.total_lut_size {
        let total_lut_size: usize = fan_curve::BUILTIN_PROFILES
            .iter()
            .map(|p| p.lut.len())
            .sum();
        println!("{total_lut_size}");
        println!("{:}", std::mem::size_of::<fan_curve::FanProfile>());
    } else if args.list_external_curves {
        let curves = fan_curve::curve_parsing::get_all_external_curves();
        info!(
            "Found {:} external curve{}{}",
            curves.len(),
            if curves.len() == 1 { "" } else { "s" },
            if curves.is_empty() { "." } else { ":" }
        );
        for curve in curves {
            println!("[OUT]: {curve}");
        }
    } else {
        let mut cmd = Args::command();
        cmd.print_help()?;
    }

    Ok(())
}

fn print_temps() -> Result<(), Box<dyn std::error::Error>> {
    let temps = temp::get_temperatures()?;
    let max_temp_idx = temps.iter().enumerate().max_by_key(|&(_, &t)| t).unwrap().0;
    println!("--- Thermal Readings ---");
    for (i, t) in temps.iter().enumerate() {
        match t.get() {
            Ok(val) => {
                println!(
                    "Sensor {i}: {:}°C{}",
                    val.to_celsius().0,
                    if i == max_temp_idx { "*" } else { "" }
                );
            }
            Err(e) => println!("Sensor {i}: {e}"),
        }
    }
    Ok(())
}

fn run_daemon(
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

fn restart_daemon<const NEW_DEFAULT: bool>(
    new_curve: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::env;
    use std::process::Command;

    let uid = env::var("SUDO_UID").unwrap_or_else(|_| String::from("1000"));
    let service_name = format!("fw-fanctrl@{uid}.service");

    info!("Applying \"{new_curve}\" curve and restarting {service_name}...");

    // ensure the new curve exists in either the built-in profiles or external curves
    let external_curves = fan_curve::curve_parsing::get_all_external_curves();
    let curve_exists = fan_curve::get_profile_by_name(new_curve, Some(&external_curves)).is_some();
    if !curve_exists {
        return Err(format!("Could not find curve \"{new_curve}\".").into());
    }

    if NEW_DEFAULT {
        // open the default config file and replace the line that specifies the default curve
        let config = std::fs::read_to_string(DEFAULT_CONFIG_PATH)?;
        let config = config.replace(
            "default_curve = \"default\"",
            &format!("default_curve = \"{new_curve}\""),
        );
        std::fs::write(DEFAULT_CONFIG_PATH, config)?;
        info!("Set \"{new_curve}\" as the new default curve.");
    } else {
        // write the curve to use once to /etc/fw-fanctrl-rs/.use-once.tmp
        let use_once_path = Path::new(USE_ONCE_PATH);
        std::fs::write(use_once_path, new_curve)?;
        infov!("Set \"{new_curve}\" as the curve to use once.");
    }

    let status = Command::new("systemctl")
        .arg("restart")
        .arg(&service_name)
        .status()?;

    if status.success() {
        info!("Daemon successfully restarted.");
        Ok(())
    } else {
        Err(format!("Failed to restart daemon. It may not be running. Try checking: systemctl status {service_name}").into())
    }
}
