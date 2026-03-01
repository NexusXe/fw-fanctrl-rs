use crate::common::{
    CrosEcCommandV2, CrosEcReadmemV2, EcCmd, FullWriteV2Command, cros_ec, cros_ec_readmem, fire,
};

use std::{
    ffi::{c_char, c_int},
    os::fd::AsRawFd,
    simd::prelude::*,
    sync::OnceLock,
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
const EC_TEMP_SENSOR_ENTRIES: usize = 16;
const SIMD_CAPABLE_TEMP_SENSORS: usize = {
    let mut x = EC_TEMP_SENSOR_ENTRIES.ilog2();
    if 2usize.pow(x) < EC_TEMP_SENSOR_ENTRIES {
        x += 1;
    }
    2usize.pow(x)
};

type TempSensorVector = Simd<u8, SIMD_CAPABLE_TEMP_SENSORS>;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct ValidEcTemp(pub(crate) u8);

impl ValidEcTemp {
    const EC_TEMP_SENSOR_DEFAULT: ValidEcTemp = ValidEcTemp((296 - EC_TEMP_SENSOR_OFFSET) as u8);

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
        ValidEcTemp(raw as u8)
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
            EcTempSensorError::NotPresent => write!(f, "Not present"),
            EcTempSensorError::Error => write!(f, "Error"),
            EcTempSensorError::NotPowered => write!(f, "Not powered"),
            EcTempSensorError::NotCalibrated => write!(f, "Not calibrated"),
        }
    }
}

impl std::error::Error for EcTempSensorError {}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct UnvalidatedEcTemp(pub(crate) u8);

impl UnvalidatedEcTemp {
    pub(crate) const fn to_celsius(self) -> Result<CelsiusTemp, EcTempSensorError> {
        let valid: ValidEcTemp =
            std::convert::Into::<Result<ValidEcTemp, EcTempSensorError>>::into(self)?;
        Ok(valid.into())
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
        KelvinTemp(u16::from(ec_temp.0) + EC_TEMP_SENSOR_OFFSET)
    }
}

impl const From<KelvinTemp> for CelsiusTemp {
    fn from(kelvin_temp: KelvinTemp) -> Self {
        CelsiusTemp(kelvin_temp.0.cast_signed() - KELVIN_CELSIUS_OFFSET.cast_signed())
    }
}

impl const From<ValidEcTemp> for CelsiusTemp {
    fn from(ec_temp: ValidEcTemp) -> Self {
        CelsiusTemp(
            u16::from(ec_temp.0).cast_signed() - EC_TEMP_SENSOR_OFFSET_CELSIUS.cast_signed(),
        )
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

#[repr(C)]
pub(crate) union TempSensorPayload {
    pub(crate) params: EcParamsTempSensorGetInfo,
    pub(crate) response: EcResponseTempSensorGetInfo,
}

type GetTempSensorInfoCommand = FullWriteV2Command<TempSensorPayload>;

pub(crate) fn probe_sensor(
    id: u8,
) -> Result<EcResponseTempSensorGetInfo, Box<dyn std::error::Error>> {
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
    let _bytes_returned: c_int = fire(&raw mut cmd.header)? // Option<NonZero<c_int>
        .ok_or("Got invalid response from temperature probe.")? // NonZero<c_int>
        .get();

    let response = unsafe { cmd.payload.response };

    Ok(response)
}

static NUM_TEMP_SENSORS: OnceLock<u8> = OnceLock::new();

pub(crate) fn num_temp_sensors() -> &'static u8 {
    NUM_TEMP_SENSORS.get_or_init(|| {
        (0..=u8::MAX)
            .take_while(|&id| probe_sensor(id).is_ok())
            .count() as u8
    })
}

fn get_temperatures_v() -> Result<TempSensorVector, nix::Error> {
    let sensors_to_read = *num_temp_sensors();
    let mut mem = CrosEcReadmemV2 {
        offset: 0x00, // EC_MEMMAP_TEMP_SENSOR
        bytes: u32::from(sensors_to_read),
        buffer: [0; 255],
    };

    unsafe {
        // Fire the v2 readmem ioctl
        let result = cros_ec_readmem(cros_ec().as_raw_fd(), &raw mut mem)?;
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
    let temps = &temps.as_array()[0..*num_temp_sensors() as _];
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
