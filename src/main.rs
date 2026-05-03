mod hid_api;
mod clipboard_api;

use clipboard_api::ClipboardApi;
use hid_api::{list_hid_devices, recv_input_report, send_output_report, HidDeviceSummary};
use std::thread;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const TIME_SEND_INTERVAL: Duration = Duration::from_secs(1);
const HOST_RX_POLL_INTERVAL: Duration = Duration::from_millis(100);
const RAW_HID_REPORT_LEN: usize = 32;
const TAG_TIME: &[u8; 4] = b"time";
const TAG_PASTE_REQ: &[u8; 4] = b"past";
const TAG_COPY_VAL: &[u8; 4] = b"copy";
const TAG_INPUT_VAL: &[u8; 4] = b"inpt";

fn main() {
    let target_name = "Trig";
    println!("Polling for device '{target_name}' every {}s.", POLL_INTERVAL.as_secs());

    let mut selected = wait_for_target_device(target_name);
    let mut clipboard = open_clipboard_with_retry();
    let mut next_time_send_at = Instant::now();

    print_selected_summary(&selected, "Selected");
    println!(
        "Sending time every {}s and handling clipboard RAW HID commands.",
        TIME_SEND_INTERVAL.as_secs()
    );

    loop {
        if Instant::now() >= next_time_send_at {
            let payload = build_time_message();
            match send_output_report(&selected, 0, &payload) {
                Ok(()) => {
                    let ts = current_unix_timestamp();
                    println!("sent time={ts}");
                }
                Err(err) => {
                    eprintln!("send failed: {err}");
                    if is_connection_error(&err) {
                        selected = wait_for_target_device(target_name);
                        print_selected_summary(&selected, "Reconnected");
                        next_time_send_at = Instant::now();
                        continue;
                    }
                }
            }

            next_time_send_at += TIME_SEND_INTERVAL;
        }

        match recv_input_report(&selected, RAW_HID_REPORT_LEN, HOST_RX_POLL_INTERVAL) {
            Ok(Some(report)) => {
                handle_keyboard_report(&selected, &mut clipboard, &report);
            }
            Ok(None) => {}
            Err(err) => {
                eprintln!("receive failed: {err}");
                if is_connection_error(&err) {
                    selected = wait_for_target_device(target_name);
                    print_selected_summary(&selected, "Reconnected");
                    next_time_send_at = Instant::now();
                }
            }
        }
    }
}

fn open_clipboard_with_retry() -> ClipboardApi {
    loop {
        match ClipboardApi::new() {
            Ok(api) => return api,
            Err(err) => {
                eprintln!("clipboard unavailable: {err}; retrying in {}s", POLL_INTERVAL.as_secs());
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

fn handle_keyboard_report(selected: &HidDeviceSummary, clipboard: &mut ClipboardApi, report: &[u8]) {
    if let Some(offset) = find_tag(report, TAG_PASTE_REQ) {
        let Some(clip_text) = clipboard.read_text().ok() else {
            eprintln!("failed to read host clipboard");
            return;
        };

        let Some(number) = parse_clipboard_number(&clip_text) else {
            eprintln!("clipboard text is not a parseable number");
            return;
        };

        let normalized = format_number_for_keyboard(number);
        let payload = build_tagged_text_message(TAG_INPUT_VAL, &normalized);
        if let Err(err) = send_output_report(selected, 0, &payload) {
            eprintln!("failed to send pasted value to keyboard: {err}");
        } else {
            println!("paste request served with value={normalized}");
        }

        let _ = offset;
        return;
    }

    if let Some(offset) = find_tag(report, TAG_COPY_VAL) {
        let payload_start = offset + TAG_COPY_VAL.len();
        let value = read_c_string(&report[payload_start..]);
        if value.is_empty() {
            return;
        }

        match clipboard.write_text(&value) {
            Ok(()) => println!("copied calculator value to host clipboard: {value}"),
            Err(err) => eprintln!("failed to write host clipboard: {err}"),
        }
    }
}

fn find_tag(data: &[u8], tag: &[u8; 4]) -> Option<usize> {
    if data.len() < tag.len() {
        return None;
    }

    data.windows(tag.len()).position(|window| window == tag)
}

fn read_c_string(data: &[u8]) -> String {
    let end = data.iter().position(|byte| *byte == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).trim().to_owned()
}

fn parse_clipboard_number(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut normalized = trimmed.replace('_', "").replace(' ', "");
    if normalized.contains(',') && normalized.contains('.') {
        normalized = normalized.replace(',', "");
    } else if normalized.contains(',') {
        normalized = normalized.replace(',', ".");
    }

    normalized.parse::<f64>().ok().filter(|n| n.is_finite())
}

fn format_number_for_keyboard(value: f64) -> String {
    let mut text = format!("{value:.12}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }

    if text == "-0" {
        "0".to_owned()
    } else {
        text
    }
}

fn build_tagged_text_message(tag: &[u8; 4], text: &str) -> Vec<u8> {
    let mut payload = vec![0u8; RAW_HID_REPORT_LEN];
    payload[..4].copy_from_slice(tag);

    let bytes = text.as_bytes();
    let copy_len = bytes.len().min(RAW_HID_REPORT_LEN.saturating_sub(5));
    payload[4..4 + copy_len].copy_from_slice(&bytes[..copy_len]);
    payload[4 + copy_len] = 0;
    payload
}

fn print_selected_summary(device: &HidDeviceSummary, prefix: &str) {
    println!(
        "{prefix} {:04x}:{:04x} iface={} class={:02x} subclass={:02x} protocol={:02x}",
        device.vendor_id,
        device.product_id,
        device.interface_number,
        device.interface_class,
        device.interface_subclass,
        device.interface_protocol
    );
}

fn print_device(device: &hid_api::HidDeviceSummary) {
    println!(
        "[{}] {:04x}:{:04x} | iface={} class={:02x} subclass={:02x} protocol={:02x} | manufacturer={} | product={} | serial={} | bus={} | address={}",
        device.index,
        device.vendor_id,
        device.product_id,
        device.interface_number,
        device.interface_class,
        device.interface_subclass,
        device.interface_protocol,
        device.manufacturer.as_deref().unwrap_or("<unknown>"),
        device.product.as_deref().unwrap_or("<unknown>"),
        device.serial_number.as_deref().unwrap_or("<unknown>"),
        device
            .bus
            .map(|bus| bus.to_string())
            .unwrap_or_else(|| "<unknown>".to_owned()),
        device
            .address
            .map(|address| address.to_string())
            .unwrap_or_else(|| "<unknown>".to_owned())
    );
}

fn choose_device(mut matched: Vec<HidDeviceSummary>) -> HidDeviceSummary {
    matched.sort_by_key(device_priority);

    if matched.len() > 1 {
        println!("Multiple matched devices found; selecting the best RAW HID candidate.");
    }

    matched.remove(0)
}

fn device_priority(device: &HidDeviceSummary) -> (u8, u8) {
    let interface_score = if device.interface_class == 0x03
        && device.interface_subclass == 0x00
        && device.interface_protocol == 0x00
    {
        0
    } else if device.interface_class == 0x03 && device.interface_protocol == 0x00 {
        1
    } else {
        2
    };

    (interface_score, device.interface_number)
}

fn wait_for_target_device(target_name: &str) -> HidDeviceSummary {
    let mut waiting_logged = false;

    loop {
        match list_hid_devices() {
            Ok(devices) => {
                let matched = filter_matching_devices(devices, target_name);
                if matched.is_empty() {
                    if !waiting_logged {
                        println!("No matching device found for '{target_name}'. Waiting...");
                        waiting_logged = true;
                    }
                    thread::sleep(POLL_INTERVAL);
                    continue;
                }

                if waiting_logged {
                    println!("Device '{target_name}' detected again.");
                }

                println!("Matched {} HID device(s) for '{target_name}':", matched.len());
                for device in &matched {
                    print_device(device);
                }

                return choose_device(matched);
            }
            Err(err) => {
                if !waiting_logged {
                    eprintln!("failed to list HID devices: {err}; retrying");
                    waiting_logged = true;
                }
                thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

fn filter_matching_devices(devices: Vec<HidDeviceSummary>, target_name: &str) -> Vec<HidDeviceSummary> {
    let target_name_lower = target_name.to_ascii_lowercase();

    devices
        .into_iter()
        .filter(|device| {
            let product = device
                .product
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let manufacturer = device
                .manufacturer
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();

            product.contains(&target_name_lower) || manufacturer.contains(&target_name_lower)
        })
        .collect()
}

fn is_connection_error(err: &str) -> bool {
    err.contains("target HID device is no longer available")
        || err.contains("failed to open target device")
        || err.contains("No such device")
        || err.contains("errno 19")
}

fn build_time_message() -> Vec<u8> {
    let ts = current_unix_timestamp();
    let mut payload = vec![0u8; RAW_HID_REPORT_LEN];
    payload[..4].copy_from_slice(TAG_TIME);
    payload[4..12].copy_from_slice(&ts.to_le_bytes());
    payload
}

fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}