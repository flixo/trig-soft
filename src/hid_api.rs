use std::time::Duration;

use nusb::descriptors::TransferType;
use nusb::transfer::{Buffer, Direction, In, Interrupt, Out, TransferError};
use nusb::transfer::{ControlOut, ControlType, Recipient};
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
                bus: Some(device.busnum()),
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
                bus: Some(device.busnum()),
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

pub fn recv_input_report(
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

fn open_target_device(target: &HidDeviceSummary) -> Result<nusb::Device, String> {
    let mut devices = nusb::list_devices().wait().map_err(|err| err.to_string())?;

    let selected = devices
        .find(|d| {
            d.vendor_id() == target.vendor_id
                && d.product_id() == target.product_id
                && Some(d.busnum()) == target.bus
                && Some(d.device_address()) == target.address
                && d.serial_number().map(ToOwned::to_owned) == target.serial_number
        })
        .ok_or_else(|| "target HID device is no longer available".to_string())?;

    selected
        .open()
        .wait()
        .map_err(|err| format!("failed to open target device: {err}"))
}
