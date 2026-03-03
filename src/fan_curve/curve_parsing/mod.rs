use crate::{
    fan_curve::{FanProfile, curve_parsing::external_curves::get_external_curve},
    info,
    temp::{CelsiusTemp, UnvalidatedEcTemp},
    warn,
};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    path::Path,
};
use xxhash_rust::xxh3::xxh3_64;

use super::flatten_points;

mod external_curves;

/// Precursor to [`FanProfile`], used to parse a .curvedef file.
/// Points are in (EC-encoded temp, PWM%)
struct ParsedCurve {
    name: String,
    points: Vec<(u8, u8)>,
    signature: u64,
}

#[derive(Debug)]
pub(crate) enum CurveParseError {
    CurveDefFormatError(String),
    IoError(std::io::Error),
}

impl const From<std::io::Error> for CurveParseError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err)
    }
}

impl const From<String> for CurveParseError {
    fn from(err: String) -> Self {
        Self::CurveDefFormatError(err)
    }
}

impl std::fmt::Display for CurveParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CurveDefFormatError(s) => write!(f, "CurveDefFormatError: {s}"),
            Self::IoError(e) => write!(f, "IoError: {e}"),
        }
    }
}

impl std::error::Error for CurveParseError {}

/// Parses a .curvedef file into a [`ParsedCurve`].
///
/// The .curvedef file format is as follows:
/// - The first line is the name of the curve.
/// - Each subsequent line is a point in the curve, in the format "temp,pwm".
/// - Points should be sorted by temperature in ascending order.
/// - Points are in (°C, PWM%) format.
/// - Comments start with '#' and are ignored.
///
/// This function will convert the °C to EC-encoded temp and return a [`ParsedCurve`],
/// or [`CurveParseError`] if the file is malformed.
fn squash_curvedef(file: File) -> Result<ParsedCurve, CurveParseError> {
    let reader = BufReader::new(file);
    let mut name: Option<String> = None;
    let mut points: Vec<(u8, u8)> = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if name.is_none() {
            name = Some(line.to_string());
            continue;
        }

        // discard anything after a #
        let line = line.split('#').next().unwrap_or(line);
        let line = line.trim();

        let mut parts = line.split(',');
        // convert both no next line or parse error into CurveParseError
        let temp = parts
            .next()
            .ok_or_else(|| {
                CurveParseError::CurveDefFormatError(format!("Missing temperature on line {i}"))
            })
            .and_then(|s| {
                s.trim().parse::<i16>().map_err(|_| {
                    CurveParseError::CurveDefFormatError(format!("Invalid temperature on line {i}"))
                })
            })?;

        let ec_temp: UnvalidatedEcTemp =
            UnvalidatedEcTemp::try_from(CelsiusTemp(temp)).map_err(|_| {
                CurveParseError::CurveDefFormatError(format!(
                    "Temperature {temp} out of EC range on line {i}"
                ))
            })?;

        if ec_temp.get().is_err() {
            return Err(CurveParseError::CurveDefFormatError(format!(
                "Nonsensical temperature {temp} on line {i}"
            )));
        }

        let pwm = parts
            .next()
            .ok_or_else(|| CurveParseError::CurveDefFormatError(format!("Missing PWM on line {i}")))
            .and_then(|s| {
                s.trim().parse::<u8>().map_err(|_| {
                    CurveParseError::CurveDefFormatError(format!("Invalid PWM on line {i}"))
                })
            })?;

        let ec_raw = ec_temp.get().unwrap();
        points.push((ec_raw.0, pwm));
    }

    let points_slice = &points;

    let signature = xxh3_64(flatten_points(points_slice));
    // Validate that points are valid:
    if points_slice.len() < 2 {
        return Err(CurveParseError::CurveDefFormatError(
            "At least two points (start and end) are required".to_string(),
        ));
    }
    if points_slice.windows(2).any(|w| w[1].0 <= w[0].0) {
        return Err(CurveParseError::CurveDefFormatError(
            "Curve X coordinates must be strictly increasing".to_string(),
        ));
    }
    if points_slice.iter().any(|p| p.1 > 100) {
        return Err(CurveParseError::CurveDefFormatError(
            "Intermediate Y must be <= 100".to_string(),
        ));
    }
    Ok(ParsedCurve {
        name: name.ok_or_else(|| {
            CurveParseError::CurveDefFormatError("No name found... somehow".to_string())
        })?,
        points,
        signature,
    })
}

/// Gets all .curvedef files in /etc/fw-fanctrl-rs/curves/ and parses them.
/// Returns a vector of [`FanProfile`]s, which may be empty if the directory doesn't exist or contains no valid curves.
/// Any invalid .curvedef files will be skipped and a warning will be printed.
pub(in super::super) fn get_all_external_curves() -> Vec<FanProfile> {
    let path = Path::new("/etc/fw-fanctrl-rs/curves/");

    if !path.exists() {
        match fs::create_dir_all(path) {
            Ok(()) => {
                info!("Created external curve directory {}", path.display());
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    unreachable!("Directory should not exist if it was just created.")
                } else {
                    warn!("Failed to create directory {}: {e}", path.display());
                }
            }
        }
        return vec![];
    }

    // If the directory itself can't be read, return an empty vec
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(e) => {
            warn!(
                "Directory {} exists but cannot be read: {e}",
                path.display()
            );
            return vec![];
        }
    };

    let mut parsed_curves = Vec::new();

    for entry in entries {
        // Handle directory entry errors
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to read directory entry: {e}");
                continue;
            }
        };

        let path = entry.path();

        // Filter for .curvedef files
        if path.is_file() && path.extension().is_some_and(|ext| ext == "curvedef") {
            let result = get_external_curve(&path);
            match result {
                Ok(curve) => parsed_curves.push(curve),
                Err(e) => {
                    warn!(
                        "Failed to parse curve at {}: {e}. Skipping.",
                        path.display()
                    );
                }
            }
        }
    }

    parsed_curves
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File, path::Path};

    #[test]
    fn test_squash_curvedef() {
        let file_path = Path::new("curves/example-curve.curvedef");
        let curve = squash_curvedef(File::open(file_path).unwrap()).unwrap();
        assert_eq!(curve.name, "ExampleCurve");
        assert_eq!(curve.points.len(), 6);
        // Points are stored as (EC-encoded temp, pwm). EC = celsius + 73.
        assert_eq!(curve.points[0], (63, 10)); // -10°C
        assert_eq!(curve.points[1], (83, 15)); //  10°C
        assert_eq!(curve.points[2], (84, 17)); //  11°C
        assert_eq!(curve.points[3], (133, 50)); //  60°C
        assert_eq!(curve.points[4], (143, 100)); //  70°C
        assert_eq!(curve.points[5], (173, 100)); // 100°C
    }
}
