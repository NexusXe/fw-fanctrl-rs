use nix::ioctl_readwrite;
use std::ffi::c_int;
use std::fs::{File, OpenOptions};
use std::num::NonZero;
use std::os::fd::AsRawFd;
use std::sync::LazyLock;

use crate::infov;

pub(crate) static CROS_EC_FILE: LazyLock<File> = LazyLock::new(|| {
    let ec = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/cros_ec")
        .expect("[ERROR]: Failed to open /dev/cros_ec. Are you running as root?");
    infov!("Got EC file handle.");
    ec
});

#[allow(dead_code)]
pub(crate) enum EcCmd {
    ProtoVersion = 0x0000,
    Hello = 0x0001,
    GetVersion = 0x0002,
    ReadTest = 0x0003,
    GetBuildInfo = 0x0004,
    GetChipInfo = 0x0005,
    GetBoardVersion = 0x0006,
    ReadMemmap = 0x0007,
    GetCmdVersions = 0x0008,
    GetCommsStatus = 0x0009,
    TestProtocol = 0x000A,
    GetProtocolInfo = 0x000B,
    GsvPauseInS5 = 0x000C,
    GetFeatures = 0x000D,
    GetSkuId = 0x000E,
    SetSkuId = 0x000F,
    FlashInfo = 0x0010,
    FlashRead = 0x0011,
    FlashWrite = 0x0012,
    FlashErase = 0x0013,
    FlashProtect = 0x0015,
    FlashRegionInfo = 0x0016,
    VbnvContext = 0x0017,
    FlashSpiInfo = 0x0018,
    FlashSelect = 0x0019,
    RandNum = 0x001A,
    RwsigInfo = 0x001B,
    Sysinfo = 0x001C,
    PwmGetFanTargetRpm = 0x0020,
    PwmSetFanTargetRpm = 0x0021,
    PwmGetKeyboardBacklight = 0x0022,
    PwmSetKeyboardBacklight = 0x0023,
    PwmSetFanDuty = 0x0024,
    PwmSetDuty = 0x0025,
    PwmGetDuty = 0x0026,
    LightbarCmd = 0x0028,
    LedControl = 0x0029,
    VbootHash = 0x002A,
    MotionSenseCmd = 0x002B,
    ForceLidOpen = 0x002C,
    ConfigPowerButton = 0x002D,
    UsbChargeSetMode = 0x0030,
    PstoreInfo = 0x0040,
    PstoreRead = 0x0041,
    PstoreWrite = 0x0042,
    RtcGetValue = 0x0044,
    RtcGetAlarm = 0x0045,
    RtcSetValue = 0x0046,
    RtcSetAlarm = 0x0047,
    Port80Read = 0x0048,
    VstoreInfo = 0x0049,
    VstoreRead = 0x004A,
    VstoreWrite = 0x004B,
    ThermalSetThreshold = 0x0050,
    ThermalGetThreshold = 0x0051,
    ThermalAutoFanCtrl = 0x0052,
    Tmp006GetCalibration = 0x0053,
    Tmp006SetCalibration = 0x0054,
    Tmp006GetRaw = 0x0055,
    MkbpState = 0x0060,
    MkbpInfo = 0x0061,
    MkbpSimulateKey = 0x0062,
    GetKeyboardId = 0x0063,
    MkbpSetConfig = 0x0064,
    MkbpGetConfig = 0x0065,
    KeyscanSeqCtrl = 0x0066,
    GetNextEvent = 0x0067,
    KeyboardFactoryTest = 0x0068,
    MkbpWakeMask = 0x0069,
    TempSensorGetInfo = 0x0070,
    AcpiRead = 0x0080,
    AcpiWrite = 0x0081,
    AcpiBurstEnable = 0x0082,
    AcpiBurstDisable = 0x0083,
    AcpiQueryEvent = 0x0084,
    HostEventGetB = 0x0087,
    HostEventGetSmiMask = 0x0088,
    HostEventGetSciMask = 0x0089,
    HostEventSetSmiMask = 0x008A,
    HostEventSetSciMask = 0x008B,
    HostEventClear = 0x008C,
    HostEventGetWakeMask = 0x008D,
    HostEventSetWakeMask = 0x008E,
    HostEventClearB = 0x008F,
    SwitchEnableBklight = 0x0090,
    SwitchEnableWireless = 0x0091,
    GpioSet = 0x0092,
    GpioGet = 0x0093,
    I2cRead = 0x0094,
    I2cWrite = 0x0095,
    ChargeControl = 0x0096,
    ConsoleSnapshot = 0x0097,
    ConsoleRead = 0x0098,
    BatteryCutOff = 0x0099,
    UsbMux = 0x009A,
    LdoSet = 0x009B,
    LdoGet = 0x009C,
    PowerInfo = 0x009D,
    I2cPassthru = 0x009E,
    HangDetect = 0x009F,
    ChargeState = 0x00A0,
    ChargeCurrentLimit = 0x00A1,
    ExternalPowerLimit = 0x00A2,
    OverrideDedicatedChargerLimit = 0x00A3,
    HostEvent = 0x00A4,
    HibernationDelay = 0x00A8,
    HostSleepEvent = 0x00A9,
    DeviceEvent = 0x00AA,
    SbReadWord = 0x00B0,
    SbWriteWord = 0x00B1,
    SbReadBlock = 0x00B2,
    SbWriteBlock = 0x00B3,
    BatteryVendorParam = 0x00B4,
    SbFwUpdate = 0x00B5,
    EnteringMode = 0x00B6,
    I2cPassthruProtect = 0x00B7,
    CecWriteMsg = 0x00B8,
    CecSet = 0x00BA,
    CecGet = 0x00BB,
    EcCodec = 0x00BC,
    EcCodecDmic = 0x00BD,
    EcCodecI2sRx = 0x00BE,
    EcCodecWov = 0x00BF,
    Pse = 0x00C0,
    Reboot = 0x00D1,
    RebootEc = 0x00D2,
    GetPanicInfo = 0x00D3,
    ResendResponse = 0x00DB,
    Version0 = 0x00DC,
    PdExchangeStatus = 0x0100,
    UsbPdControl = 0x0101,
    UsbPdPorts = 0x0102,
    UsbPdPowerInfo = 0x0103,
    PdHostEventStatus = 0x0104,
    ChargePortCount = 0x0105,
    UsbPdFwUpdate = 0x0110,
    UsbPdRwHashEntry = 0x0111,
    UsbPdDevInfo = 0x0112,
    UsbPdDiscovery = 0x0113,
    PdChargePortOverride = 0x0114,
    PdGetLogEntry = 0x0115,
    UsbPdGetAmode = 0x0116,
    UsbPdSetAmode = 0x0117,
    PdWriteLogEntry = 0x0118,
    PdControl = 0x0119,
    UsbPdMuxInfo = 0x011A,
    PdChipInfo = 0x011B,
    RwsigCheckStatus = 0x011C,
    RwsigAction = 0x011D,
    EfsVerify = 0x011E,
    GetCrosBoardInfo = 0x011F,
    SetCrosBoardInfo = 0x0120,
    GetUptimeInfo = 0x0121,
    AddEntropy = 0x0122,
    AdcRead = 0x0123,
    RollbackInfo = 0x0124,
    ApReset = 0x0125,
    LocateChip = 0x0126,
    RebootApOnG3 = 0x0127,
    GetPdPortCaps = 0x0128,
    Button = 0x0129,
    GetKeybdConfig = 0x012A,
    SmartDischarge = 0x012B,
    RegulatorGetInfo = 0x012C,
    RegulatorEnable = 0x012D,
    RegulatorIsEnabled = 0x012E,
    RegulatorSetVoltage = 0x012F,
    RegulatorGetVoltage = 0x0130,
    TypecDiscovery = 0x0131,
    TypecControl = 0x0132,
    TypecStatus = 0x0133,
    Cr51Base = 0x0300,
    Cr51Last = 0x03FF,
    FpPassthru = 0x0400,
    FpMode = 0x0402,
    FpInfo = 0x0403,
    FpFrame = 0x0404,
    FpTemplate = 0x0405,
    FpContext = 0x0406,
    FpStats = 0x0407,
    FpSeed = 0x0408,
    FpEncStatus = 0x0409,
    FpReadMatchSecret = 0x040A,
    TpSelfTest = 0x0500,
    TpFrameInfo = 0x0501,
    TpFrameSnapshot = 0x0502,
    TpFrameGet = 0x0503,
    BatteryGetStatic = 0x0600,
    BatteryGetDynamic = 0x0601,
    ChargerControl = 0x0602,
    BoardSpecificBase = 0x3E00,
    BoardSpecificLast = 0x3FFF,
}

#[repr(C)]
pub(crate) struct CrosEcCommandV2 {
    pub(crate) version: u32 = 0,
    pub(crate) command: u32,
    pub(crate) outsize: u32,
    pub(crate) insize: u32,
    pub(crate) result: u32 = 0,
    pub(crate) data: [u8; 0] = [],
}

#[repr(C)]
pub(crate) struct FullWriteV2Command<T> {
    pub(crate) header: CrosEcCommandV2,
    pub(crate) payload: T,
}

const EC_MEMMAP_SIZE: usize = 255;

#[repr(C)]
pub(crate) struct CrosEcReadmemV2 {
    pub(crate) offset: u32,
    pub(crate) bytes: u32,
    pub(crate) buffer: [u8; EC_MEMMAP_SIZE],
}

const CROS_EC_MAGIC: u8 = 0xEC;
const CROS_EC_DEV_IOCXCMD: c_int = 0;
const CROS_EC_DEV_IOCRDMEM_V2: c_int = 1;

ioctl_readwrite!(
    cros_ec_cmd,
    CROS_EC_MAGIC,
    CROS_EC_DEV_IOCXCMD,
    CrosEcCommandV2
);

ioctl_readwrite!(
    cros_ec_readmem,
    CROS_EC_MAGIC,
    CROS_EC_DEV_IOCRDMEM_V2,
    CrosEcReadmemV2
);

pub(crate) fn fire(payload: *mut CrosEcCommandV2) -> Result<Option<NonZero<c_int>>, nix::Error> {
    unsafe {
        let result = cros_ec_cmd(CROS_EC_FILE.as_raw_fd(), payload)?;
        if result < 0 {
            Err(nix::Error::from_raw(result))
        } else {
            Ok(NonZero::<c_int>::new(result))
        }
    }
}
