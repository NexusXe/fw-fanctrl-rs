#![feature(default_field_values)]
#![feature(generic_const_exprs)]
#![feature(const_cmp)]
#![feature(const_trait_impl)]
#![feature(const_convert)]
#![feature(const_default)]
#![allow(incomplete_features)]
#![warn(clippy::pedantic)]
#![allow(clippy::cast_possible_truncation)]

pub(crate) mod common;
mod fan_curve;
mod fans;
mod temp;

use clap::Parser;
use std::convert::Into;

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

    /// Check temps and set fans to match curve once
    #[arg(short = 'o', long)]
    once: bool,

    /// Print fan curve in CSV format
    #[arg(long)]
    curve: bool,

    /// Fan curve profile to use
    #[arg(short = 'p', long, default_value = "default")]
    profile: String,

    /// Generate shell completions
    #[arg(long, value_enum)]
    print_completions: Option<clap_complete::Shell>,

    /// Print total LUT size
    #[arg(long)]
    total_lut_size: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use clap::CommandFactory;
    let args = Args::parse();

    if let Some(shell) = args.print_completions {
        let mut cmd = Args::command();
        clap_complete::generate(shell, &mut cmd, "fw-fanctrl-rs", &mut std::io::stdout());
        return Ok(());
    }

    let profile = fan_curve::get_profile_by_name(&args.profile).unwrap_or_else(|| {
        eprintln!("Profile '{}' not found, using default.", args.profile);
        fan_curve::get_profile_by_name("default").unwrap()
    });

    if args.temp {
        let temps = temp::get_temperatures()?;
        let max_temp_idx = temps.iter().enumerate().max_by_key(|&(_, &t)| t).unwrap().0;
        println!("--- Thermal Readings ---");
        for (i, t) in temps.iter().enumerate() {
            match t.get() {
                Ok(val) => {
                    //let celsius = i32::from(val) + 200 - 273;
                    println!(
                        "Sensor {i}: {:}°C ({val:}, 0b{val:08b}){}",
                        val - 73,
                        if i == max_temp_idx { "*" } else { "" }
                    );
                }
                Err(e) => println!("Sensor {i}: {e}"),
            }
        }
    } else if let Some(val) = args.fan {
        if val == "auto" {
            fans::set_auto()?;
            println!("Set auto fan control.");
        } else {
            let duty: u8 = val.parse::<u8>()?.clamp(0, 100);
            fans::set_duty(duty)?;
            println!("Set to {duty:}");
        }
    } else if args.once {
        // check temps and set fans to match curve
        let max_temp = temp::get_max_temp()?;
        let speed = profile.get_fan_speed(max_temp);
        fans::set_duty(speed)?;
        println!(
            "{:}°C: {speed:3}%",
            Into::<Result<temp::CelsiusTemp, temp::EcTempSensorError>>::into(max_temp)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    temp::CelsiusTemp::default()
                })
                .0
        );
    } else if args.daemon {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        ctrlc::set_handler(move || {
            r.store(false, Ordering::SeqCst);
        })
        .expect("Error setting Ctrl-C handler");

        while running.load(Ordering::SeqCst) {
            let max_temp = temp::get_max_temp()?;
            let speed = profile.get_fan_speed(max_temp);
            fans::set_duty(speed)?;
            println!(
                "{:}°C: {speed:3}%",
                Into::<Result<temp::CelsiusTemp, temp::EcTempSensorError>>::into(max_temp)
                    .unwrap_or_else(|e| {
                        eprintln!("{e}");
                        temp::CelsiusTemp::default()
                    })
                    .0
            );
            std::thread::sleep(std::time::Duration::from_secs(1));
        }

        // Cleanup
        println!("\nShutting down...");
        fans::set_auto()?;
        println!("Set auto fan control.");
    } else if args.curve {
        println!("Temperature,PWM");
        for temp in 0..=u8::MAX - 4 {
            let temp = temp::EcTemp(temp);
            println!(
                "{:},{:}",
                Into::<Result<temp::CelsiusTemp, temp::EcTempSensorError>>::into(temp)
                    .unwrap_or_else(|e| {
                        eprintln!("{e}");
                        temp::CelsiusTemp::default()
                    })
                    .0,
                profile.get_fan_speed(temp)
            );
        }
    } else if args.total_lut_size {
        let total_lut_size: usize = fan_curve::PROFILES.iter().map(|p| p.lut.len()).sum();
        println!("{total_lut_size}");
    } else {
        let mut cmd = Args::command();
        cmd.print_help()?;
    }

    Ok(())
}
