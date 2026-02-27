#![feature(default_field_values)]
#![feature(generic_const_exprs)]
#![allow(incomplete_features)]
#![warn(clippy::pedantic)]
#![allow(clippy::cast_possible_truncation)]

pub(crate) mod common;
mod fans;
mod temp;
mod fan_curve;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args();
    if let Some(arg) = args.nth(1) {
        match arg.as_str() {
            "fans" => {
                if let Some(val) = args.next() {
                    let duty: u8 = val.parse::<u8>()?.clamp(0, 100);
                    fans::set_duty(duty)?;
                    println!("Set to {duty:}");
                } else {
                    fans::set_auto()?;
                    println!("Set auto fan control.");
                }
            }

            "temp" => {
                let temps = temp::get_temperatures()?;
                let max_temp_idx = temps.iter().enumerate().max_by_key(|&(_, &t)| t).unwrap().0;
                println!("--- Thermal Readings ---");
                for (i, t) in temps.iter().enumerate() {
                    match t {
                        0xFF => println!("Sensor {i}: Not present"),
                        0xFE => println!("Sensor {i}: Error"),
                        0xFD => println!("Sensor {i}: Not powered"),
                        0xFC => println!("Sensor {i}: Not calibrated"),
                        val => {
                            //let celsius = i32::from(val) + 200 - 273;
                            println!(
                                "Sensor {i}: {:}°C ({val:}, 0b{val:08b}){}",
                                val - 73,
                                if i == max_temp_idx { "*" } else { "" }
                            );
                        }
                    }
                }
            }

            "dwell" => {
                // check temps and set fans to match curve
                let max_temp = temp::get_max_temp()?;
                let speed = fan_curve::FAN_LUT[max_temp as usize];
                fans::set_duty(speed)?;
                println!("{:}°C: {speed:3}%", max_temp - 73);
            }

            "daemon" => {
                use std::sync::atomic::{AtomicBool, Ordering};
                use std::sync::Arc;

                let running = Arc::new(AtomicBool::new(true));
                let r = running.clone();
                ctrlc::set_handler(move || {
                    r.store(false, Ordering::SeqCst);
                }).expect("Error setting Ctrl-C handler");

                while running.load(Ordering::SeqCst) {
                    let max_temp = temp::get_max_temp()?;
                    let speed = fan_curve::FAN_LUT[max_temp as usize];
                    fans::set_duty(speed)?;
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }

                // Cleanup
                fans::set_auto()?;
                println!("Set auto fan control.");
            }

            _ => {
                // for (temp, speed) in fan_curve::FAN_LUT.iter().enumerate() {
                //     let temp = (temp as i64 - 273) + i64::from(temp::EC_TEMP_SENSOR_OFFSET);
                //     println!("{temp:3}°C ({:3}F): {speed:3}%", temp * 9 / 5 + 32);
                // }
                // return Ok(());
                println!("Temperature,PWM");
                for temp in 0..=u8::MAX {
                    println!("{:},{:}", i16::from(temp) - 73, fan_curve::FAN_LUT[temp as usize]);
                }
            }
        }
    }
    Ok(())
}
