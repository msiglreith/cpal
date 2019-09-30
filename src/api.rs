//!

use error::BackendSpecificError;
use std::{error, fmt, result};
use Format;

#[derive(Debug)]
pub enum Error {
    DeviceLost,
    Unsupported,

    Io(std::io::Error),
    BackendSpecific { description: String },
}

impl error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        match *self {
            Error::DeviceLost => writeln!(f, "Device Lost"),
            Error::Unsupported => writeln!(f, "Unsupported"),
            Error::Io(ref err) => writeln!(f, "IO: {}", err),
            Error::BackendSpecific { ref description } => {
                writeln!(f, "Backend Specific: {}", description)
            }
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

impl From<BackendSpecificError> for Error {
    fn from(err: BackendSpecificError) -> Self {
        Error::BackendSpecific {
            description: err.description,
        }
    }
}

pub type Result<T> = result::Result<T, Error>;

pub type Frames = usize;

#[derive(Debug, Clone)]
pub struct PhysicalDeviceProperties {
    pub device_name: String,
}

#[derive(Debug, Clone)]
pub struct DeviceProperties {}

pub trait Instance {
    type PhysicalDevice: PhysicalDevice;
    type Device: Device;

    fn create(name: &str) -> Self;

    fn enumerate_physical_input_devices(&self) -> Result<Vec<Self::PhysicalDevice>>;

    fn enumerate_physical_output_devices(&self) -> Result<Vec<Self::PhysicalDevice>>;

    /// The default input audio device on the system.
    ///
    /// Returns `None` if no input device is available.
    fn default_physical_input_device(&self) -> Result<Option<Self::PhysicalDevice>>;

    /// The default output audio device on the system.
    ///
    /// Returns `None` if no output device is available.
    fn default_physical_output_device(&self) -> Result<Option<Self::PhysicalDevice>>;

    fn create_device(
        &self,
        physical_device: &Self::PhysicalDevice,
        format: Format,
    ) -> Result<Self::Device>;
}

pub trait PhysicalDevice {
    fn properties(&self) -> PhysicalDeviceProperties;
}

pub trait Device {
    type OutputStream: OutputStream;
    type InputStream: InputStream;

    fn properties(&self) -> DeviceProperties;

    fn output_stream(&self) -> Result<Self::OutputStream>;
    fn async_output_stream(&self) -> Result<Self::OutputStream>;

    fn input_stream(&self) -> Result<Self::InputStream>;
    fn async_input_stream(&self) -> Result<Self::InputStream>;
}

pub trait OutputStream {
    fn start(&self);
    fn stop(&self);

    fn acquire_buffer(&self, timeout_ms: u32) -> (*mut (), Frames);
    fn release_buffer(&self, num_frames: Frames);
}

pub trait InputStream {}
