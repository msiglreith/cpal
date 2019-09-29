use std::ptr;

use super::com::{self, WeakPtr};
use super::winapi::shared::devpkey::*;
use super::winapi::shared::ksmedia;
use super::winapi::shared::mmreg::*;
use super::winapi::um::audioclient::*;
use super::winapi::um::audiosessiontypes::*;
use super::winapi::um::combaseapi::*;
use super::winapi::um::coml2api::STGM_READ;
use super::winapi::um::mmdeviceapi::*;
use super::winapi::um::objbase::COINIT_MULTITHREADED;
use super::winapi::um::propsys::*;
use super::winapi::um::synchapi;
use super::winapi::um::winnt;
use super::winapi::Interface;
use super::check_result;
use {traits, Format};

pub type Instance = WeakPtr<IMMDeviceEnumerator>;

impl traits::Instance for Instance {
    type PhysicalDevice = PhysicalDevice;
    type Device = Device;

    fn create(_: &str) -> Self {
        // COM initialization is thread local, but we only need to have COM initialized in the
        // thread we create the objects in
        com::com_initialized();

        let mut instance = Instance::null();
        let hresult = unsafe {
            CoCreateInstance(
                &CLSID_MMDeviceEnumerator,
                ptr::null_mut(),
                CLSCTX_ALL,
                &IMMDeviceEnumerator::uuidof(),
                instance.mut_void(),
            )
        };
        check_result(hresult).unwrap();

        instance
    }

    fn enumerate_physical_input_devices(&self) -> Vec<Self::PhysicalDevice> {
        unimplemented!()
    }

    fn create_device(
        &self,
        physical_device: &Self::PhysicalDevice,
        format: Format,
    ) -> Self::Device {
        unimplemented!()
    }
}

pub type PhysicalDevice = WeakPtr<IMMDevice>;

impl traits::PhysicalDevice for PhysicalDevice {
    fn properties(&self) -> traits::PhysicalDeviceProperties {
        unimplemented!()
    }
}

pub struct Device {
    audio_client: WeakPtr<IAudioClient>,
    fence: Fence,
}

impl traits::Device for Device {
    type OutputStream = OutputStream;
    type InputStream = InputStream;

    fn properties(&self) -> traits::DeviceProperties {
        unimplemented!()
    }

    fn output_stream(&self) -> Self::OutputStream {
        unimplemented!()
    }

    fn async_output_stream(&self) -> Self::OutputStream {
        unimplemented!()
    }

    fn input_stream(&self) -> Self::InputStream {
        unimplemented!()
    }

    fn async_input_stream(&self) -> Self::InputStream {
        unimplemented!()
    }
}

pub struct OutputStream {
    audio_client: WeakPtr<IAudioClient>,
    audio_render_client: WeakPtr<IAudioRenderClient>,
    fence: Fence,
}

impl traits::OutputStream for OutputStream {
    fn start(&self) {
        unimplemented!()
    }

    fn stop(&self) {
        unimplemented!()
    }

    fn acquire_buffer(&self, timeout_ms: u32) -> (*mut (), traits::Frames) {
        unimplemented!()
    }

    fn release_buffer(&self, num_frames: traits::Frames) {
        unimplemented!()
    }
}

pub struct InputStream {
    audio_capture_client: WeakPtr<IAudioCaptureClient>,
    fence: Fence,
}

impl traits::InputStream for InputStream {}

#[derive(Copy, Clone)]
struct Fence(pub winnt::HANDLE);
impl Fence {
    unsafe fn create(manual_reset: bool, initial_state: bool) -> Self {
        Fence(synchapi::CreateEventA(
            ptr::null_mut(),
            manual_reset as _,
            initial_state as _,
            ptr::null(),
        ))
    }

    unsafe fn wait(&self, timeout_ms: u32) -> u32 {
        synchapi::WaitForSingleObject(self.0, timeout_ms)
    }
}
