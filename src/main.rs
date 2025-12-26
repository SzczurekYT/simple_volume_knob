#![no_std]
#![no_main]

pub mod bluetooth;

use async_debounce::Debouncer;
use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_rp::{
    Peri, bind_interrupts,
    gpio::{AnyPin, Input, Level, Output, Pull},
    peripherals::{DMA_CH0, PIO0},
    pio::{InterruptHandler, Pio},
};
use embassy_time::Duration;
use embedded_hal::digital::InputPin;
use embedded_hal_async::digital::Wait;
use static_cell::StaticCell;
use trouble_host::prelude::ExternalController;

use {defmt_rtt as _, panic_probe as _};

const MASK: u8 = 0b111;
const LEFT_P1: u8 = 0b100;
const LEFT_P2: u8 = 0b110;
const LEFT_P1_INV: u8 = 0b011;
const LEFT_P2_INV: u8 = 0b001;
const RIGHT_P1: u8 = 0b110;
const RIGHT_P2: u8 = 0b100;
const RIGHT_P1_INV: u8 = 0b001;
const RIGHT_P2_INV: u8 = 0b011;

const DEBOUNCE_MS: u64 = 1;

const CYW43_FW: &[u8] = include_bytes!("../cyw43-firmware/43439A0.bin");
const CYW43_CLM: &[u8] = include_bytes!("../cyw43-firmware/43439A0_clm.bin");
const CYW43_BTFW: &[u8] = include_bytes!("../cyw43-firmware/43439A0_btfw.bin");

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        cyw43_pio::DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH0,
    );

    static CYW43_STATE: StaticCell<cyw43::State> = StaticCell::new();
    let cyw43_state = CYW43_STATE.init(cyw43::State::new());
    let (_net_device, bt_device, mut control, runner) =
        cyw43::new_with_bluetooth(cyw43_state, pwr, spi, CYW43_FW, CYW43_BTFW).await;
    spawner.spawn(cyw43_task(runner)).unwrap();
    control.init(CYW43_CLM).await;

    let bt_controller: ExternalController<_, 10> = ExternalController::new(bt_device);

    bluetooth::run_bluetooth(bt_controller).await;

    spawner
        .spawn(knob_controller(p.PIN_16.into(), p.PIN_17.into()))
        .unwrap();
}

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn knob_controller(p1: Peri<'static, AnyPin>, p2: Peri<'static, AnyPin>) {
    let mut in1 = Debouncer::new(Input::new(p1, Pull::Up), Duration::from_millis(DEBOUNCE_MS));
    let mut in2 = Debouncer::new(Input::new(p2, Pull::Up), Duration::from_millis(DEBOUNCE_MS));

    let mut in1_history: u8 = in1.is_high().unwrap() as u8;
    let mut in2_history: u8 = in2.is_high().unwrap() as u8;

    loop {
        // Infallible errors
        let _ = select(in1.wait_for_any_edge(), in2.wait_for_any_edge()).await;
        in1_history <<= 1;
        in1_history |= in1.is_high().unwrap() as u8;
        in2_history <<= 1;
        in2_history |= in2.is_high().unwrap() as u8;

        let in1_pattern = in1_history & MASK;
        let in2_pattern = in2_history & MASK;

        if in1_pattern == LEFT_P1 && in2_pattern == LEFT_P2
            || in1_pattern == LEFT_P1_INV && in2_pattern == LEFT_P2_INV
        {
            info!("Rot left");
        } else if in1_pattern == RIGHT_P1 && in2_pattern == RIGHT_P2
            || in1_pattern == RIGHT_P1_INV && in2_pattern == RIGHT_P2_INV
        {
            info!("Rot right");
        }
    }
}
