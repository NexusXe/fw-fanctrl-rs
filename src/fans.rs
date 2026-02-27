use crate::common::{CrosEcCommandV2, EcCmd, FullWriteV2Command, fire};

#[repr(C, align(4))]
struct EcParamsPwmSetFanDuty {
    percent: u32,
}

type SetFanDutyCommand = FullWriteV2Command<EcParamsPwmSetFanDuty>;

pub(crate) fn set_duty(percent: u8) -> Result<(), nix::Error> {
    let mut cmd = SetFanDutyCommand {
        header: CrosEcCommandV2 {
            command: EcCmd::PwmSetFanDuty as u32,
            outsize: std::mem::size_of::<EcParamsPwmSetFanDuty>() as u32,
            insize: 0,
            ..
        },
        payload: EcParamsPwmSetFanDuty {
            percent: u32::from(percent),
        },
    };
    let _ = fire(&raw mut cmd.header)?;
    Ok(())
}

pub(crate) fn set_auto() -> Result<(), nix::Error> {
    let mut cmd = CrosEcCommandV2 {
        command: EcCmd::ThermalAutoFanCtrl as u32,
        outsize: 0,
        insize: 0,
        ..
    };
    let _ = fire(&raw mut cmd)?;
    Ok(())
}
