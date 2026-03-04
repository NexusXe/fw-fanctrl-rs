use crate::{
    common::{
        CROS_EC_FILE, CrosEcCommandV2, CrosEcReadmemV2, EcCmd, FullWriteV2Command, cros_ec_readmem,
        fire,
    },
    infov,
};

use std::{
    ffi::{CStr, c_char},
    fmt::Display,
    os::fd::AsRawFd,
    simd::prelude::*,
    sync::{LazyLock, OnceLock},
};

/// The offset of temperature value stored in mapped memory.  This allows
/// reporting a temperature range of 200K to 454K = -73C to 181C.
pub(crate) const EC_TEMP_SENSOR_OFFSET: u16 = 200;
pub(crate) const KELVIN_CELSIUS_OFFSET: u16 = 273;
pub(crate) const EC_TEMP_SENSOR_OFFSET_CELSIUS: u16 = KELVIN_CELSIUS_OFFSET - EC_TEMP_SENSOR_OFFSET;

#[allow(unused)]
pub(crate) const MIN_TEMP_CELSIUS: i16 = -73;
#[allow(unused)]
pub(crate) const MAX_TEMP_CELSIUS: i16 = 181;

/// Number of temp sensors at `EC_MEMMAP_TEMP_SENSOR`
pub(crate) const EC_TEMP_SENSOR_ENTRIES: usize = 16;
const SIMD_CAPABLE_TEMP_SENSORS: usize = {
    let mut x = EC_TEMP_SENSOR_ENTRIES.ilog2();
    if 2usize.pow(x) < EC_TEMP_SENSOR_ENTRIES {
        x += 1;
    }
    2usize.pow(x)
};

pub(crate) type TempSensorVector = Simd<u8, SIMD_CAPABLE_TEMP_SENSORS>;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct ValidEcTemp(pub(crate) u8);

impl ValidEcTemp {
    const EC_TEMP_SENSOR_DEFAULT: Self = Self((296 - EC_TEMP_SENSOR_OFFSET) as u8);

    pub(crate) const fn to_celsius(self) -> CelsiusTemp {
        self.into()
    }

    #[allow(dead_code)]
    pub(crate) const fn from_celsius(celsius: CelsiusTemp) -> Self {
        celsius.into()
    }
}

impl const std::default::Default for ValidEcTemp {
    fn default() -> Self {
        Self::EC_TEMP_SENSOR_DEFAULT
    }
}

impl const From<CelsiusTemp> for ValidEcTemp {
    #[inline]
    fn from(celsius: CelsiusTemp) -> Self {
        let raw = celsius.0 + EC_TEMP_SENSOR_OFFSET_CELSIUS.cast_signed();
        // verify that the conversion is valid
        debug_assert!(raw >= 0 && raw <= 255);
        #[allow(clippy::cast_sign_loss)]
        Self(raw as u8)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EcTempSensorError {
    NotPresent = 0xFF,
    Error = 0xFE,
    NotPowered = 0xFD,
    NotCalibrated = 0xFC,
}

impl std::fmt::Display for EcTempSensorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotPresent => write!(f, "Not present"),
            Self::Error => write!(f, "Error"),
            Self::NotPowered => write!(f, "Not powered"),
            Self::NotCalibrated => write!(f, "Not calibrated"),
        }
    }
}

impl std::error::Error for EcTempSensorError {}

#[repr(transparent)]
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct UnvalidatedEcTemp(pub(crate) u8);

impl UnvalidatedEcTemp {
    pub(crate) const fn to_celsius(self) -> Result<CelsiusTemp, EcTempSensorError> {
        let valid: ValidEcTemp =
            std::convert::Into::<Result<ValidEcTemp, EcTempSensorError>>::into(self)?;
        Ok(valid.into())
    }

    #[allow(dead_code)] // used by plugins
    pub(crate) const fn validate(self) -> Result<ValidEcTemp, EcTempSensorError> {
        self.into()
    }
}

impl const From<UnvalidatedEcTemp> for Result<ValidEcTemp, EcTempSensorError> {
    fn from(val: UnvalidatedEcTemp) -> Self {
        match val.0 {
            0xFF => Err(EcTempSensorError::NotPresent),
            0xFE => Err(EcTempSensorError::Error),
            0xFD => Err(EcTempSensorError::NotPowered),
            0xFC => Err(EcTempSensorError::NotCalibrated),
            _ => Ok(ValidEcTemp(val.0)),
        }
    }
}

impl UnvalidatedEcTemp {
    pub(crate) const fn get(self) -> Result<ValidEcTemp, EcTempSensorError> {
        self.into()
    }
}

impl const Default for UnvalidatedEcTemp {
    fn default() -> Self {
        Self(0x00)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct KelvinTemp(pub(crate) u16);

impl const Default for KelvinTemp {
    fn default() -> Self {
        Self(EC_TEMP_SENSOR_OFFSET)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CelsiusTemp(pub(crate) i16);

impl const Default for CelsiusTemp {
    fn default() -> Self {
        KelvinTemp::default().into()
    }
}

impl const From<ValidEcTemp> for KelvinTemp {
    fn from(ec_temp: ValidEcTemp) -> Self {
        Self(u16::from(ec_temp.0) + EC_TEMP_SENSOR_OFFSET)
    }
}

impl const From<KelvinTemp> for CelsiusTemp {
    fn from(kelvin_temp: KelvinTemp) -> Self {
        Self(kelvin_temp.0.cast_signed() - KELVIN_CELSIUS_OFFSET.cast_signed())
    }
}

impl const From<ValidEcTemp> for CelsiusTemp {
    fn from(ec_temp: ValidEcTemp) -> Self {
        Self(u16::from(ec_temp.0).cast_signed() - EC_TEMP_SENSOR_OFFSET_CELSIUS.cast_signed())
    }
}

impl TryFrom<CelsiusTemp> for UnvalidatedEcTemp {
    type Error = &'static str;
    fn try_from(celsius: CelsiusTemp) -> Result<Self, Self::Error> {
        let raw = celsius.0 + EC_TEMP_SENSOR_OFFSET_CELSIUS.cast_signed();
        u8::try_from(raw)
            .map(UnvalidatedEcTemp)
            .map_err(|_| "CelsiusTemp out of range for EcTemp")
    }
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub(crate) struct EcParamsTempSensorGetInfo {
    pub(crate) id: u8,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub(crate) struct EcResponseTempSensorGetInfo {
    pub(crate) sensor_name: [c_char; 32],
    pub(crate) sensor_type: u8,
}

impl Display for EcResponseTempSensorGetInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name_cstr = unsafe { CStr::from_ptr(self.sensor_name.as_ptr()) };
        let name = name_cstr.to_str().unwrap_or("<invalid UTF-8>");

        let sensor_type = match self.sensor_type {
            255 => "TEMP_SENSOR_TYPE_IGNORED",
            0 => "TEMP_SENSOR_TYPE_CPU",
            1 => "TEMP_SENSOR_TYPE_BOARD",
            2 => "TEMP_SENSOR_TYPE_CASE",
            3 => "TEMP_SENSOR_TYPE_BATTERY",
            4 => "TEMP_SENSOR_TYPE_COUNT",
            _ => "ERROR: BAD SENSOR TYPE",
        };

        write!(f, "{name} ({sensor_type})")
    }
}

impl const Default for EcResponseTempSensorGetInfo {
    fn default() -> Self {
        Self {
            sensor_name: [0; 32],
            sensor_type: 255,
        }
    }
}

#[repr(C)]
pub(crate) union TempSensorPayload {
    pub(crate) params: EcParamsTempSensorGetInfo,
    pub(crate) response: EcResponseTempSensorGetInfo,
}

type GetTempSensorInfoCommand = FullWriteV2Command<TempSensorPayload>;

/// Cache of sensor info responses
pub(crate) static SENSOR_CACHE: [OnceLock<EcResponseTempSensorGetInfo>; EC_TEMP_SENSOR_ENTRIES] =
    [const { OnceLock::new() }; EC_TEMP_SENSOR_ENTRIES];

pub(crate) fn probe_sensor(
    id: u8,
) -> Result<EcResponseTempSensorGetInfo, Box<dyn std::error::Error>> {
    if id >= EC_TEMP_SENSOR_ENTRIES as u8 {
        return Err("Invalid sensor ID".into());
    }

    let info = SENSOR_CACHE[id as usize].get_or_try_init(|| {
        let mut cmd = GetTempSensorInfoCommand {
            header: CrosEcCommandV2 {
                command: EcCmd::TempSensorGetInfo as u32,
                outsize: std::mem::size_of::<EcParamsTempSensorGetInfo>() as u32,
                insize: std::mem::size_of::<EcResponseTempSensorGetInfo>() as u32,
                ..
            },
            payload: TempSensorPayload {
                params: EcParamsTempSensorGetInfo { id },
            },
        };

        let _bytes_returned: std::ffi::c_int = fire(&raw mut cmd.header)?
            .ok_or("Got invalid response from temperature probe.")?
            .get();

        Ok::<_, Box<dyn std::error::Error>>(unsafe { cmd.payload.response })
    })?;

    Ok(*info)
}

pub(crate) static NUM_TEMP_SENSORS: LazyLock<u8> = LazyLock::new(|| {
    let num = (0..=u8::MAX)
        .take_while(|&id| probe_sensor(id).is_ok())
        .count() as u8;
    infov!("Got {num:} temperature sensors.");
    num
});

pub(crate) fn get_temperatures_v() -> Result<TempSensorVector, nix::Error> {
    let sensors_to_read = *NUM_TEMP_SENSORS;
    let mut mem = CrosEcReadmemV2 {
        offset: 0x00, // EC_MEMMAP_TEMP_SENSOR
        bytes: u32::from(sensors_to_read),
        buffer: [0; 255],
    };

    unsafe {
        // Fire the v2 readmem ioctl
        let result = cros_ec_readmem(CROS_EC_FILE.as_raw_fd(), &raw mut mem)?;
        if result < 0 {
            return Err(nix::Error::from_raw(result));
        }
    }

    Ok(TempSensorVector::from_slice(
        &mem.buffer[..TempSensorVector::LEN],
    ))
}

pub(crate) fn get_temperatures() -> Result<Vec<UnvalidatedEcTemp>, nix::Error> {
    let temps = get_temperatures_v()?;
    let temps = &temps.as_array()[0..*NUM_TEMP_SENSORS as _];
    Ok(temps.iter().map(|&t| UnvalidatedEcTemp(t)).collect())
}

#[inline]
fn max_temp(input: TempSensorVector) -> ValidEcTemp {
    ValidEcTemp(input.reduce_max())
}

pub(crate) fn get_max_temp() -> Result<ValidEcTemp, nix::Error> {
    let temps = get_temperatures_v()?;
    Ok(max_temp(temps))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ec_temp_to_kelvin() {
        use std::convert::Into;

        let valid_cases = [0, 50, 100, 0xFB];
        for case in valid_cases {
            let res: Result<ValidEcTemp, EcTempSensorError> = UnvalidatedEcTemp(case).into();
            assert!(res.is_ok());
            let res: KelvinTemp = res.unwrap().into();

            assert_eq!(res.0, u16::from(case) + EC_TEMP_SENSOR_OFFSET);
        }

        assert_eq!(
            Into::<Result<ValidEcTemp, EcTempSensorError>>::into(UnvalidatedEcTemp(0xFF)),
            Err(EcTempSensorError::NotPresent)
        );
        assert_eq!(
            Into::<Result<ValidEcTemp, EcTempSensorError>>::into(UnvalidatedEcTemp(0xFE)),
            Err(EcTempSensorError::Error)
        );
        assert_eq!(
            Into::<Result<ValidEcTemp, EcTempSensorError>>::into(UnvalidatedEcTemp(0xFD)),
            Err(EcTempSensorError::NotPowered)
        );
        assert_eq!(
            Into::<Result<ValidEcTemp, EcTempSensorError>>::into(UnvalidatedEcTemp(0xFC)),
            Err(EcTempSensorError::NotCalibrated)
        );
    }

    #[test]
    fn test_kelvin_to_celsius() {
        let test_cases = [(273, 0), (300, 27), (200, -73), (0, -273)];

        for (kelvin, expected_celsius) in test_cases {
            let celsius: CelsiusTemp = KelvinTemp(kelvin).into();
            assert_eq!(celsius.0, expected_celsius);
        }
    }
}
