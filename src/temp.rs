use crate::common::{
    CrosEcCommandV2, CrosEcReadmemV2, EcCmd, FullWriteV2Command, cros_ec, cros_ec_readmem, fire,
};

use std::{
    ffi::{c_char, c_int},
    os::fd::AsRawFd,
    sync::OnceLock,
};

/// The offset of temperature value stored in mapped memory.  This allows
/// reporting a temperature range of 200K to 454K = -73C to 181C.
pub(crate) const EC_TEMP_SENSOR_OFFSET: u32 = 200;

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

pub(crate) fn get_temperatures() -> Result<Vec<u8>, nix::Error> {
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

    Ok(mem.buffer[..sensors_to_read as usize].to_vec())
}

fn maxima_native(input: &[u8]) -> u8 {
    unsafe { *input.iter().max().unwrap_unchecked() }
}

#[target_feature(enable = "avx512vl")]
#[allow(clippy::cast_sign_loss)]
unsafe fn maxima_vl(input: &[u8]) -> u8 {
    use std::arch::x86_64::{
        __m128i, _mm_cvtepu8_epi16, _mm_cvtsi128_si32, _mm_loadu_si64, _mm_minpos_epu16,
        _mm_ternarylogic_epi32,
    };
    unsafe {
        let mut v: __m128i = _mm_loadu_si64(input.as_ptr());
        v = _mm_ternarylogic_epi32(v, v, v, 0x55);
        v = _mm_cvtepu8_epi16(v);
        v = _mm_minpos_epu16(v);
        !_mm_cvtsi128_si32(v) as u8
    }
}

pub(crate) fn max_temp(input: &[u8]) -> u8 {
    if cfg!(target_feature = "avx512vl") && input.len() == 8 {
        unsafe { maxima_vl(input) }
    } else {
        maxima_native(input)
    }
}

pub(crate) fn get_max_temp() -> Result<u8, nix::Error> {
    let temps = get_temperatures()?;
    Ok(max_temp(&temps))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maxima_consistency() {
        // Simple Linear Congruential Generator for deterministic RNG
        struct Lcg {
            state: u32,
        }

        impl Lcg {
            fn new(seed: u32) -> Self {
                Self { state: seed }
            }

            fn next_u8(&mut self) -> u8 {
                self.state = self
                    .state
                    .wrapping_mul(1_664_525)
                    .wrapping_add(1_013_904_223);
                (self.state >> 24) as u8
            }
        }

        // if !std::is_x86_feature_detected!("avx512vl") && !std::is_x86_feature_detected!("avx512bw")
        // {
        //     if !std::is_x86_feature_detected!("avx512f")
        //         || !std::is_x86_feature_detected!("avx512vl")
        //     {
        //         println!("Skipping test: avx512f/avx512vl not supported on this CPU");
        //         return;
        //     }
        // } else if !std::is_x86_feature_detected!("avx512vl") {
        //     println!("Skipping test: avx512vl not supported on this CPU");
        //     return;
        // }

        let mut rng = Lcg::new(42);

        for _ in 0..2usize.pow(22) {
            let mut input = [0u8; 8];
            for byte in &mut input {
                *byte = rng.next_u8();
            }

            let expected = maxima_native(&input);
            let actual = unsafe { maxima_vl(&input) };

            assert_eq!(
                expected, actual,
                "Mismatch on input {input:?}: native={expected}, vl={actual}"
            );
        }
    }
}
