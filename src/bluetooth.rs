use defmt::{panic, *};
use embassy_futures::{join::join, select::select};
use embassy_time::Timer;
use trouble_host::prelude::*;
use {defmt_rtt as _, panic_probe as _};

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 2; // Signal + att <-- copied from example, no idea what means

const NAME: &str = "Simple Volume Knob";

#[gatt_server]
struct Server {
    battery_service: BatteryService,
    _device_info: DeviceInformationService,
    hid: HidService,
}

#[gatt_service(uuid = service::BATTERY)]
struct BatteryService {
    /// Battery Level
    #[descriptor(uuid = descriptors::VALID_RANGE, read, value = [0, 100])]
    #[descriptor(uuid = descriptors::MEASUREMENT_DESCRIPTION, name = "hello", read, value = "Battery Level")]
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 10)]
    level: u8,
    #[characteristic(uuid = "408813df-5dd4-1f87-ec11-cdb001100000", write, read, notify)]
    status: bool,
}

const MANFUCATURER: [u8; 7] = *b"RatLabs";
const MODEL_NUMBER_DATA: [u8; 7] = *b"SVK-1.0";

#[gatt_service(uuid = service::DEVICE_INFORMATION)]
struct DeviceInformationService {
    #[characteristic(uuid = characteristic::MANUFACTURER_NAME_STRING, read, value = MANFUCATURER)]
    manufacturer_name: [u8; 7],
    #[characteristic(uuid = characteristic::MODEL_NUMBER_STRING, read, value = MODEL_NUMBER_DATA)]
    model_number: [u8; 7],
}

const REPORT_DESCRIPTOR: [u8; 29] = [
    0x05, 0x0C, 0x09, 0x01, 0xA1, 0x01, 0x15, 0x00, 0x25, 0x01, 0x95, 0x03, 0x75, 0x01, 0x09, 0xE9,
    0x09, 0xEA, 0x09, 0xE2, 0x81, 0x02, 0x95, 0x01, 0x75, 0x05, 0x81, 0x01, 0xC0,
];

#[gatt_service(uuid = service::HUMAN_INTERFACE_DEVICE)]
struct HidService {
    #[characteristic(uuid = characteristic::HID_INFORMATION, read, value = [0x01, 0x01, 0x00, 0x03])]
    hid_info: [u8; 4],
    #[characteristic(uuid = characteristic::REPORT_MAP, read, value = REPORT_DESCRIPTOR)]
    report_map: [u8; 29],
    #[characteristic(uuid = characteristic::HID_CONTROL_POINT, write_without_response)]
    hid_control_point: u8,
    #[characteristic(uuid = characteristic::PROTOCOL_MODE, read, write_without_response, value = 1)]
    protocol_mode: u8,
    #[descriptor(uuid = descriptors::REPORT_REFERENCE, read, value = [0u8, 1u8])]
    #[characteristic(uuid = characteristic::REPORT, read, notify)]
    input: u8,
}

pub async fn run_bluetooth<C: Controller>(controller: C) {
    let address: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    info!("Our address = {:?}", address);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources).set_random_address(address);
    let Host {
        mut peripheral,
        runner,
        ..
    } = stack.build();

    info!("Starting advertising and GATT service");

    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: NAME,
        appearance: &appearance::control_device::ROTARY_SWITCH,
    }))
    .unwrap();

    let _ = join(ble_task(runner), async {
        loop {
            match advertise(NAME, &mut peripheral, &server).await {
                Ok(conn) => {
                    // set up tasks when the connection is established to a central, so they don't run when no one is connected.
                    let a = gatt_events_task(&server, &conn);
                    let b = custom_task(&server, &conn);
                    // run until any task ends (usually because the connection has been closed),
                    // then return to advertising state.
                    select(a, b).await;
                }
                Err(e) => {
                    let e = defmt::Debug2Format(&e);
                    panic!("[adv] error: {:?}", e);
                }
            }
        }
    })
    .await;
}

async fn ble_task<C: Controller, P: PacketPool>(mut runner: Runner<'_, C, P>) {
    loop {
        if let Err(e) = runner.run().await {
            let e = defmt::Debug2Format(&e);
            panic!("[ble_task] error: {:?}", e);
        }
    }
}

/// Stream Events until the connection closes.
///
/// This function will handle the GATT events and process them.
/// This is how we interact with read and write requests.
async fn gatt_events_task<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
) -> Result<(), Error> {
    let level = server.battery_service.level;
    let reason = loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => break reason,
            GattConnectionEvent::Gatt { event } => {
                match &event {
                    GattEvent::Read(event) => {
                        if event.handle() == level.handle {
                            let value = server.get(&level);
                            info!("[gatt] Read Event to Level Characteristic: {:?}", value);
                        }
                    }
                    GattEvent::Write(event) => {
                        if event.handle() == level.handle {
                            info!(
                                "[gatt] Write Event to Level Characteristic: {:?}",
                                event.data()
                            );
                        }
                    }
                    _ => {}
                };
                // This step is also performed at drop(), but writing it explicitly is necessary
                // in order to ensure reply is sent.
                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("[gatt] error sending response: {:?}", e),
                };
            }
            _ => {} // ignore other Gatt Connection Events
        }
    };
    info!("[gatt] disconnected: {:?}", reason);
    Ok(())
}

async fn advertise<'values, 'server, C: Controller>(
    name: &'values str,
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
    server: &'server Server<'values>,
) -> Result<GattConnection<'values, 'server, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut advertiser_data = [0; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteLocalName(name.as_bytes()),
            AdStructure::ServiceUuids16(&[
                service::HUMAN_INTERFACE_DEVICE.to_le_bytes(),
                service::BATTERY.to_le_bytes(),
            ]),
        ],
        &mut advertiser_data[..],
    )?;
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &advertiser_data[..len],
                scan_data: &[],
            },
        )
        .await?;
    info!("[adv] advertising");
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    info!("[adv] connection established");
    Ok(conn)
}

async fn custom_task<P: PacketPool>(server: &Server<'_>, conn: &GattConnection<'_, '_, P>) {
    let mut toggle = true;
    let report = server.hid.input;
    loop {
        let report_value = if toggle { &0b0000_0100 } else { &0b0000_0000 };
        if report.notify(conn, report_value).await.is_err() {
            info!("[custom_task] error notifying connection");
            break;
        };
        toggle = !toggle;
        Timer::after_secs(2).await;
    }
}
