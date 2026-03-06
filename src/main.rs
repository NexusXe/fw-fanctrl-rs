#![feature(default_field_values)]
#![feature(generic_const_exprs)]
#![feature(const_trait_impl)]
#![feature(const_convert)]
#![feature(const_default)]
#![feature(const_try)]
#![feature(portable_simd)]
#![feature(once_cell_try)]
#![feature(optimize_attribute)]
#![allow(incomplete_features)]
#![warn(clippy::nursery)]
#![allow(clippy::redundant_pub_crate)]
#![allow(clippy::cast_possible_truncation)]

#[cfg(target_family = "windows")]
compile_error!(
    "fw-fanctrl-rs does not support Windows. Consider http://ozturkkl.github.io/framework-control/ for a Windows-compatible alternative.\nNote that I have never used it, so I cannot vouch for its quality."
);

pub(crate) mod common;
mod daemon;
mod fan_curve;
mod fans;
#[cfg(feature = "plot")]
mod plot;
mod temp;

use clap::{CommandFactory, Parser};
use serde::Deserialize;
use std::{
    fs,
    num::NonZeroU64,
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[derive(Deserialize)]
struct Config {
    default_curve: String,
    poll_interval_ms: u64,
    #[allow(dead_code)] // only used when plugin feature is enabled
    plugin_path: Option<String>,
}

const DEFAULT_CONFIG_PATH: &str = "/etc/fw-fanctrl-rs/config.toml";
const USE_ONCE_PATH: &str = "/tmp/fw-fanctrl-rs.use-once.tmp";

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

    /// Set fan speeds (0-100 or 'auto').
    /// Default: auto
    #[arg(short = 'f', long, value_name = "SPEED", num_args = 0..=1, default_missing_value = "auto")]
    fan: Option<String>,

    /// Run as daemon
    #[arg(short = 'd', long, conflicts_with = "once")]
    daemon: bool,

    /// Sleep duration in milliseconds between checks
    /// Default: 1000ms, or config file's poll_interval_ms
    #[arg(short = 's', long)]
    sleep_millis: Option<NonZeroU64>,

    /// Check temps and set fans to match curve once
    #[arg(short = 'O', long)]
    once: bool,

    /// Print fan curve in CSV format
    #[arg(long)]
    curve: bool,

    /// Fan curve profile to use
    /// Default behavior: if /tmp/fw-fanctrl-rs.use-once.tmp exists, use it, otherwise use "default"
    #[arg(short = 'p', long)]
    profile: Option<String>,

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

    /// Restart the daemon using a custom curve, also setting it as the new default.
    ///
    /// This will overwrite the default curve in the **default** config file at /etc/fw-fanctrl-rs/config.toml.
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

    #[cfg(feature = "plot")]
    /// Plot all curves
    #[arg(
        short = 'P',
        long,
        conflicts_with = "once",
        conflicts_with = "daemon",
        conflicts_with = "fan",
        conflicts_with = "temp",
        conflicts_with = "curve",
        conflicts_with = "total_lut_size",
        conflicts_with = "list_external_curves",
        conflicts_with = "use",
        conflicts_with = "use_default"
    )]
    plot: bool,

    #[cfg(feature = "plot")]
    #[allow(clippy::doc_markdown)]
    /// Path to save the plot to.
    /// Supported formats: png, jpg, webp, svg
    /// Default: ./fan_curves.png
    #[arg(short = 'o', long, requires("plot"), default_value = "fan_curves.png")]
    out: String,

    #[cfg(feature = "plot")]
    /// (Try to) force sixel output
    #[arg(long, requires("plot"))]
    force_sixel: bool,

    #[cfg(feature = "plot")]
    /// (Try to) force kitty output
    #[arg(long, requires("plot"))]
    force_kitty: bool,

    #[arg(short = 'e', long, requires("daemon"))]
    plugin: Option<String>,
}

#[allow(clippy::too_many_lines)] // too bad!
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.quiet {
        QUIET.set(true).unwrap();
    } else if args.verbose {
        // mutually exclusive with quiet
        VERBOSE.set(true).unwrap();
    }

    #[cfg(not(feature = "plugin"))]
    if args.plugin.is_some() {
        return Err("[ERROR]: fw-fanctrl-rs was not built with plugin support.".into());
    }

    let mut profiles = fan_curve::curve_parsing::get_all_external_curves();
    profiles.extend(fan_curve::BUILTIN_PROFILES.iter().cloned());

    if let Some(profile) = args.r#use {
        daemon::restart_daemon::<false>(&profile, &profiles)?;
        return Ok(());
    } else if let Some(profile) = args.use_default {
        daemon::restart_daemon::<true>(&profile, &profiles)?;
        return Ok(());
    }

    let mut config_default = None;
    let mut config_sleep_millis: Option<NonZeroU64> = None;
    #[allow(unused_mut)] // used by plugins
    let mut config_plugin: Option<String> = None;

    match load_config(&args.config) {
        Ok(config) => {
            infov!("Loaded config from {}", args.config);
            config_default = Some(config.default_curve);
            infov!(
                "    With default profile: {}",
                config_default.as_ref().unwrap()
            );
            config_sleep_millis = Some(
                NonZeroU64::new(config.poll_interval_ms)
                    .expect("[ERROR]: Config cannot have 0ms poll interval"),
            );
            infov!("    With poll interval: {}ms", config_sleep_millis.unwrap());
            #[cfg(feature = "plugin")]
            if let Some(plugin_path) = config.plugin_path {
                config_plugin = Some(plugin_path);
                infov!("    With plugin: {}", config_plugin.as_ref().unwrap());
            } else {
                infov!("    No plugin specified in config");
            }
        }
        Err(e) => {
            warn!("Failed to load config: {e}");
        }
    }

    let sleep_millis: NonZeroU64 = args
        .sleep_millis
        .or(config_sleep_millis)
        .unwrap_or_else(|| NonZeroU64::new(1000).unwrap()); // unwrap is fine here since 1000 != 0

    let plugin_path = args.plugin.or(config_plugin).map(PathBuf::from);
    let plugin: Option<&PathBuf> = plugin_path.as_ref();

    let config_default = config_default.unwrap_or_else(|| "default".to_string());

    let profile_name = args.profile.as_ref().map_or_else(
        || {
            let use_once_path = Path::new(USE_ONCE_PATH);
            if use_once_path.exists() {
                infov!("Use-once file found, using profile from file.");
                let p = match std::fs::read_to_string(use_once_path) {
                    Ok(content) => content.trim().to_string(),
                    Err(e) => {
                        warn!("Failed to read use-once file: {e}");
                        "default".to_string()
                    }
                };
                let _ = std::fs::remove_file(use_once_path);
                p
            } else {
                infov!("No use-once file found, using profile from config.");
                config_default
            }
        },
        |p| {
            infov!("Using profile from command line: {}", p);
            p.clone()
        },
    );

    if let Some(shell) = args.print_completions {
        let mut cmd = Args::command();
        clap_complete::generate(shell, &mut cmd, "fw-fanctrl-rs", &mut std::io::stdout());
        return Ok(());
    }

    let profile = fan_curve::get_profile_by_name(&profile_name, &profiles)
        .unwrap_or_else(|| {
            warn!("Profile '{}' not found, using default.", profile_name);
            fan_curve::get_profile_by_name("default", &profiles).expect("Default profile not found")
        })
        .to_owned();

    #[cfg(feature = "plot")]
    if args.plot {
        let path = Path::new(&args.out);
        return plot::plot_curves(path, &profiles, args.force_sixel, args.force_kitty);
    }

    // explicitly drop the rest of the profiles
    drop(profiles);

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
        daemon::run_daemon(&profile, sleep_millis, plugin).map_err(|e| {
            eprintln!("[ERROR]: {e}");
            e
        })?;
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
