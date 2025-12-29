use crate::hid;
use defmt::{panic, *};
use embassy_futures::{join::join, select::select};
use embassy_time::Timer;
use rand_core::{CryptoRng, RngCore};
use trouble_host::prelude::*;

use {defmt_rtt as _, panic_probe as _};

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 4;

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
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 100)]
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

#[derive(Debug, Clone, Copy)]
enum KeyPressed {
    VolUp,
    VolDown,
    Mute,
    None,
}

type InputRaport = [u8; 2];

impl KeyPressed {
    pub fn as_report(&self) -> InputRaport {
        let value = match self {
            KeyPressed::VolUp => 0b0000_0001,
            KeyPressed::VolDown => 0b0000_0010,
            KeyPressed::Mute => 0b0000_0100,
            KeyPressed::None => 0b0000_0000,
        };
        [hid::HID_REPORT_INPUT_ID, value]
    }

    pub async fn send<P: PacketPool>(
        &self,
        conn: &GattConnection<'_, '_, P>,
        server: &Server<'_>,
    ) -> Result<(), trouble_host::Error> {
        let report = server.hid.input;

        report.notify(conn, &self.as_report()).await?;

        Timer::after_millis(50).await;

        report.notify(conn, &KeyPressed::None.as_report()).await
    }
}

#[gatt_service(uuid = service::HUMAN_INTERFACE_DEVICE)]
struct HidService {
    #[characteristic(uuid = characteristic::HID_INFORMATION, read, value = [0x01, 0x01, 0x00, 0x03])]
    hid_info: [u8; 4],
    #[characteristic(uuid = characteristic::REPORT_MAP, read, value = hid::HID_REPORT_DESCRIPTOR)]
    report_map: [u8; 31],
    #[characteristic(uuid = characteristic::HID_CONTROL_POINT, write_without_response)]
    hid_control_point: u8,
    #[characteristic(uuid = characteristic::PROTOCOL_MODE, read, write_without_response, value = 1)]
    protocol_mode: u8,
    #[descriptor(uuid = descriptors::REPORT_REFERENCE, read, value = [0u8, hid::HID_REPORT_INPUT_ID])]
    #[characteristic(uuid = characteristic::REPORT, read, notify, value = [hid::HID_REPORT_INPUT_ID, 0u8])]
    input: InputRaport,
}

pub async fn run_bluetooth<C, RNG>(controller: C, mut rng: RNG)
where
    C: Controller,
    RNG: RngCore + CryptoRng,
{
    let mut bond_info: Option<BondInformation> = None;

    let address: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    info!("Device address = {:?}", address);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .set_random_generator_seed(&mut rng)
        .set_io_capabilities(IoCapabilities::DisplayYesNo);

    let Host {
        mut peripheral,
        runner,
        ..
    } = stack.build();

    info!("Starting advertising and GATT service");

    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: NAME,
        appearance: &appearance::human_interface_device::KEYBOARD,
    }))
    .unwrap();

    let _ = join(ble_task(runner), async {
        loop {
            match advertise(NAME, &mut peripheral, &server).await {
                Ok(conn) => {
                    conn.raw().set_bondable(bond_info.is_none()).unwrap();

                    let a = gatt_events_task(&server, &conn, &mut bond_info);
                    let b = custom_task(&server, &conn);

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

async fn gatt_events_task<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    bond_info: &mut Option<BondInformation>,
) -> Result<(), Error> {
    let reason = loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => break reason,
            GattConnectionEvent::PassKeyDisplay(key) => {
                info!("[gatt] passkey display: {}", key);
            }
            GattConnectionEvent::PassKeyConfirm(_) => {
                // Always confirm
                info!("[auth] PassKeyConfirm event");
                conn.pass_key_confirm()?;
            }
            GattConnectionEvent::PassKeyInput => {
                info!("[auth] PassKeyInput event");
            }

            GattConnectionEvent::PairingComplete {
                security_level,
                bond,
            } => {
                info!(
                    "[auth] pairing complete: {:?}, bond: {:?}",
                    security_level, bond
                );
                *bond_info = bond;
            }
            GattConnectionEvent::PairingFailed(err) => {
                error!("[auth] pairing error: {:?}", err);
            }
            GattConnectionEvent::Gatt { event } => handle_gatt_event(event, server, conn).await?,
            _ => {}
        }
    };
    info!("[gatt] disconnected: {:?}", reason);
    Ok(())
}

async fn handle_gatt_event<P: PacketPool>(
    event: GattEvent<'_, '_, P>,
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
) -> Result<(), Error> {
    let level = server.battery_service.level;
    let result = match &event {
        GattEvent::Read(event) => {
            if event.handle() == level.handle {
                let value = server.get(&level);
                info!("[gatt] Read Event to Level Characteristic: {:?}", value);
            }
            if conn.raw().security_level()?.authenticated() {
                None
            } else {
                Some(AttErrorCode::INSUFFICIENT_AUTHENTICATION)
            }
        }
        GattEvent::Write(event) => {
            if event.handle() == level.handle {
                info!(
                    "[gatt] Write Event to Level Characteristic: {:?}",
                    event.data()
                );
            }
            if conn.raw().security_level()?.authenticated() {
                None
            } else {
                Some(AttErrorCode::INSUFFICIENT_AUTHENTICATION)
            }
        }
        _ => None,
    };

    let reply_result = if let Some(code) = result {
        event.reject(code)
    } else {
        event.accept()
    };
    match reply_result {
        Ok(reply) => reply.send().await,
        Err(e) => warn!("[gatt] error sending response: {:?}", e),
    }
    Ok(())
}

async fn custom_task<P: PacketPool>(server: &Server<'_>, conn: &GattConnection<'_, '_, P>) {
    let mut toggle = true;
    loop {
        let key = if toggle {
            KeyPressed::VolUp
        } else {
            KeyPressed::VolDown
        };
        if key.send(conn, server).await.is_err() {
            info!("[custom_task] error notifying connection");
            break;
        };
        toggle = !toggle;
        Timer::after_secs(2).await;
    }
}
