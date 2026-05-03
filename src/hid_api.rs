use std::time::Duration;

#[cfg(target_os = "windows")]
use hidapi::HidApi;
#[cfg(target_os = "windows")]
use std::cell::RefCell;
#[cfg(not(target_os = "windows"))]
use nusb::descriptors::TransferType;
#[cfg(not(target_os = "windows"))]
use nusb::transfer::{Buffer, Direction, In, Interrupt, Out, TransferError};
#[cfg(not(target_os = "windows"))]
use nusb::transfer::{ControlOut, ControlType, Recipient};
#[cfg(not(target_os = "windows"))]
use nusb::MaybeFuture;

pub struct HidDeviceSummary {
    pub index: usize,
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
    pub bus: Option<u8>,
    pub address: Option<u8>,
    pub interface_number: u8,
    pub interface_class: u8,
    pub interface_subclass: u8,
    pub interface_protocol: u8,
}

#[cfg(target_os = "windows")]
struct CachedHidDevice {
    vendor_id: u16,
    product_id: u16,
    serial_number: Option<String>,
    interface_number: u8,
    device: hidapi::HidDevice,
}

#[cfg(target_os = "windows")]
thread_local! {
    static HID_DEVICE_CACHE: RefCell<Option<CachedHidDevice>> = const { RefCell::new(None) };
}

fn device_bus(device: &nusb::DeviceInfo) -> Option<u8> {
    #[cfg(target_os = "linux")]
    {
        Some(device.busnum())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = device;
        None
    }
}

pub fn list_hid_devices() -> Result<Vec<HidDeviceSummary>, String> {
    let mut hid_devices = Vec::new();

    let devices = pollster::block_on(nusb::list_devices()).map_err(|err| err.to_string())?;

    for device in devices {
        let mut has_hid_interface = false;

        for interface in device.interfaces() {
            if interface.class() != 0x03 {
                continue;
            }

            has_hid_interface = true;
            hid_devices.push(HidDeviceSummary {
                index: hid_devices.len(),
                vendor_id: device.vendor_id(),
                product_id: device.product_id(),
                manufacturer: device.manufacturer_string().map(ToOwned::to_owned),
                product: device.product_string().map(ToOwned::to_owned),
                serial_number: device.serial_number().map(ToOwned::to_owned),
                bus: device_bus(&device),
                address: Some(device.device_address()),
                interface_number: interface.interface_number(),
                interface_class: interface.class(),
                interface_subclass: interface.subclass(),
                interface_protocol: interface.protocol(),
            });
        }

        if !has_hid_interface && device.class() == 0x03 {
            hid_devices.push(HidDeviceSummary {
                index: hid_devices.len(),
                vendor_id: device.vendor_id(),
                product_id: device.product_id(),
                manufacturer: device.manufacturer_string().map(ToOwned::to_owned),
                product: device.product_string().map(ToOwned::to_owned),
                serial_number: device.serial_number().map(ToOwned::to_owned),
                bus: device_bus(&device),
                address: Some(device.device_address()),
                interface_number: 0,
                interface_class: 0x03,
                interface_subclass: 0,
                interface_protocol: 0,
            });
        }
    }

    Ok(hid_devices)
}

pub fn send_output_report(
    target: &HidDeviceSummary,
    report_id: u8,
    payload: &[u8],
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        return send_output_report_windows(target, report_id, payload);
    }

    #[cfg(not(target_os = "windows"))]
    {
        send_output_report_nusb(target, report_id, payload)
    }
}

#[cfg(not(target_os = "windows"))]
fn send_output_report_nusb(
    target: &HidDeviceSummary,
    report_id: u8,
    payload: &[u8],
) -> Result<(), String> {
    const HID_SET_REPORT: u8 = 0x09;
    const HID_REPORT_TYPE_OUTPUT: u16 = 0x02;

    let device = open_target_device(target)?;
    let interface = device
        .detach_and_claim_interface(target.interface_number)
        .wait()
        .map_err(|err| format!("failed to claim interface {}: {err}", target.interface_number))?;

    if let Some(descriptor) = interface.descriptor() {
        let out_endpoint = descriptor
            .endpoints()
            .find(|endpoint| {
                endpoint.transfer_type() == TransferType::Interrupt
                    && endpoint.direction() == Direction::Out
            })
            .map(|endpoint| endpoint.address());

        if let Some(endpoint_address) = out_endpoint {
            let mut endpoint = interface
                .endpoint::<Interrupt, Out>(endpoint_address)
                .map_err(|err| format!("failed to open endpoint 0x{endpoint_address:02x}: {err}"))?;

            let completion = endpoint.transfer_blocking(payload.to_vec().into(), Duration::from_millis(1000));
            return completion
                .status
                .map_err(|err| format!("interrupt OUT transfer failed: {err}"));
        }
    }

    interface
        .control_out(
            ControlOut {
                control_type: ControlType::Class,
                recipient: Recipient::Interface,
                request: HID_SET_REPORT,
                value: (HID_REPORT_TYPE_OUTPUT << 8) | u16::from(report_id),
                index: u16::from(target.interface_number),
                data: payload,
            },
            Duration::from_millis(1000),
        )
        .wait()
        .map_err(|err| format!("HID SET_REPORT failed: {err}"))
}

#[cfg(target_os = "windows")]
fn send_output_report_windows(
    target: &HidDeviceSummary,
    report_id: u8,
    payload: &[u8],
) -> Result<(), String> {
    with_hid_device(target, |device| {
        let mut packet = Vec::with_capacity(payload.len() + 1);
        packet.push(report_id);
        packet.extend_from_slice(payload);

        let written = device
            .write(&packet)
            .map_err(|err| format!("hid write failed: {err}"))?;

        if written == packet.len() {
            Ok(())
        } else {
            Err(format!(
                "hid write was partial: wrote {written} of {} bytes",
                packet.len()
            ))
        }
    })
}

pub fn recv_input_report(
    target: &HidDeviceSummary,
    report_len: usize,
    timeout: Duration,
) -> Result<Option<Vec<u8>>, String> {
    #[cfg(target_os = "windows")]
    {
        return recv_input_report_windows(target, report_len, timeout);
    }

    #[cfg(not(target_os = "windows"))]
    {
        recv_input_report_nusb(target, report_len, timeout)
    }
}

#[cfg(not(target_os = "windows"))]
fn recv_input_report_nusb(
    target: &HidDeviceSummary,
    report_len: usize,
    timeout: Duration,
) -> Result<Option<Vec<u8>>, String> {
    let device = open_target_device(target)?;
    let interface = device
        .detach_and_claim_interface(target.interface_number)
        .wait()
        .map_err(|err| format!("failed to claim interface {}: {err}", target.interface_number))?;

    let descriptor = interface
        .descriptor()
        .ok_or_else(|| "missing interface descriptor".to_string())?;

    let in_endpoint_addr = descriptor
        .endpoints()
        .find(|endpoint| {
            endpoint.transfer_type() == TransferType::Interrupt
                && endpoint.direction() == Direction::In
        })
        .map(|endpoint| endpoint.address())
        .ok_or_else(|| "no interrupt IN endpoint found".to_string())?;

    let mut endpoint = interface
        .endpoint::<Interrupt, In>(in_endpoint_addr)
        .map_err(|err| format!("failed to open endpoint 0x{in_endpoint_addr:02x}: {err}"))?;

    let max_packet = endpoint.max_packet_size();
    let mut req_len = report_len.max(max_packet);
    if req_len % max_packet != 0 {
        req_len += max_packet - (req_len % max_packet);
    }

    let completion = endpoint.transfer_blocking(Buffer::new(req_len), timeout);
    match completion.status {
        Ok(()) => {
            let mut data = completion.buffer.into_vec();
            if completion.actual_len < data.len() {
                data.truncate(completion.actual_len);
            }

            if data.is_empty() {
                Ok(None)
            } else {
                Ok(Some(data))
            }
        }
        Err(TransferError::Cancelled) => Ok(None),
        Err(err) => Err(format!("interrupt IN transfer failed: {err}")),
    }
}

#[cfg(target_os = "windows")]
fn recv_input_report_windows(
    target: &HidDeviceSummary,
    report_len: usize,
    timeout: Duration,
) -> Result<Option<Vec<u8>>, String> {
    with_hid_device(target, |device| {
        let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let mut packet = vec![0u8; report_len.saturating_add(1)];

        let read_len = device
            .read_timeout(&mut packet, timeout_ms)
            .map_err(|err| format!("hid read failed: {err}"))?;

        if read_len == 0 {
            return Ok(None);
        }

        let mut data = packet[..read_len].to_vec();

        if !data.is_empty() && data[0] == 0 {
            data.remove(0);
        }

        if data.len() > report_len {
            data.truncate(report_len);
        }

        if data.is_empty() {
            Ok(None)
        } else {
            Ok(Some(data))
        }
    })
}

pub fn can_claim_interface(target: &HidDeviceSummary) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _device = open_target_hid_device(target)?;
        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        can_claim_interface_nusb(target)
    }
}

#[cfg(not(target_os = "windows"))]
fn can_claim_interface_nusb(target: &HidDeviceSummary) -> Result<(), String> {
    let device = open_target_device(target)?;
    let _claimed = device
        .detach_and_claim_interface(target.interface_number)
        .wait()
        .map_err(|err| format!("failed to claim interface {}: {err}", target.interface_number))?;

    Ok(())
}

#[cfg(target_os = "windows")]
fn open_target_hid_device(target: &HidDeviceSummary) -> Result<hidapi::HidDevice, String> {
    let api = HidApi::new().map_err(|err| format!("failed to initialize HID API: {err}"))?;

    let serial_matches = |candidate: Option<&str>| match (&target.serial_number, candidate) {
        (Some(expected), Some(actual)) => actual == expected,
        (None, _) => true,
        (Some(_), None) => false,
    };

    for info in api.device_list() {
        if info.vendor_id() != target.vendor_id || info.product_id() != target.product_id {
            continue;
        }

        if info.interface_number() != i32::from(target.interface_number) {
            continue;
        }

        if !serial_matches(info.serial_number()) {
            continue;
        }

        return info
            .open_device(&api)
            .map_err(|err| format!("failed to open HID interface {}: {err}", target.interface_number));
    }

    Err(format!(
        "target HID interface {} is no longer available",
        target.interface_number
    ))
}

#[cfg(target_os = "windows")]
fn cached_device_matches(cache: &CachedHidDevice, target: &HidDeviceSummary) -> bool {
    cache.vendor_id == target.vendor_id
        && cache.product_id == target.product_id
        && cache.interface_number == target.interface_number
        && cache.serial_number == target.serial_number
}

#[cfg(target_os = "windows")]
fn with_hid_device<T>(
    target: &HidDeviceSummary,
    f: impl FnOnce(&hidapi::HidDevice) -> Result<T, String>,
) -> Result<T, String> {
    HID_DEVICE_CACHE.with(|slot| {
        let mut cache = slot.borrow_mut();

        let must_open = match cache.as_ref() {
            Some(existing) => !cached_device_matches(existing, target),
            None => true,
        };

        if must_open {
            let device = open_target_hid_device(target)?;
            *cache = Some(CachedHidDevice {
                vendor_id: target.vendor_id,
                product_id: target.product_id,
                serial_number: target.serial_number.clone(),
                interface_number: target.interface_number,
                device,
            });
        }

        let Some(entry) = cache.as_ref() else {
            return Err("HID cache unexpectedly empty".to_string());
        };

        match f(&entry.device) {
            Ok(value) => Ok(value),
            Err(err) => {
                let lower = err.to_ascii_lowercase();
                if lower.contains("no such device")
                    || lower.contains("not connected")
                    || lower.contains("device not found")
                    || lower.contains("disconnected")
                {
                    *cache = None;
                }

                Err(err)
            }
        }
    })
}

#[cfg(not(target_os = "windows"))]
fn open_target_device(target: &HidDeviceSummary) -> Result<nusb::Device, String> {
    let mut devices = nusb::list_devices().wait().map_err(|err| err.to_string())?;

    let selected = devices
        .find(|d| {
            d.vendor_id() == target.vendor_id
                && d.product_id() == target.product_id
                && device_bus(d) == target.bus
                && Some(d.device_address()) == target.address
                && d.serial_number().map(ToOwned::to_owned) == target.serial_number
        })
        .ok_or_else(|| "target HID device is no longer available".to_string())?;

    selected
        .open()
        .wait()
        .map_err(|err| format!("failed to open target device: {err}"))
}
