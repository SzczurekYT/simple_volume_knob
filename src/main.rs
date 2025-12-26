#![no_std]
#![no_main]

use async_debounce::Debouncer;
use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_rp::gpio::{Input, Pull};
use embassy_time::Duration;
use embedded_hal::digital::InputPin;
use embedded_hal_async::digital::Wait;

use {defmt_rtt as _, panic_probe as _};

// Left
// P1 1 -> 0 -> 0 -> 1 -> 1
// P2 1 -> 1 -> 0 -> 0 -> 1
// Right
// P1 1 -> 1 -> 0 -> 0 -> 1
// P2 1 -> 0 -> 0 -> 1 -> 1

const MASK: u8 = 0b1111;
const LEFT_P1: u8 = 0b1001;
const LEFT_P2: u8 = 0b1100;
const RIGHT_P1: u8 = 0b1100;
const RIGHT_P2: u8 = 0b1001;

const DEBOUNCE_MS: u64 = 1;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let mut in1 = Debouncer::new(
        Input::new(p.PIN_16, Pull::Up),
        Duration::from_millis(DEBOUNCE_MS),
    );
    let mut in2 = Debouncer::new(
        Input::new(p.PIN_17, Pull::Up),
        Duration::from_millis(DEBOUNCE_MS),
    );

    let mut in1_history: u8 = in1.is_high().unwrap() as u8;
    let mut in2_history: u8 = in2.is_high().unwrap() as u8;

    loop {
        // Infallible errors
        let _ = select(in1.wait_for_any_edge(), in2.wait_for_any_edge()).await;
        in1_history <<= 1;
        in1_history |= in1.is_high().unwrap() as u8;
        in2_history <<= 1;
        in2_history |= in2.is_high().unwrap() as u8;

        let in1_last_four = in1_history & MASK;
        let in2_last_four = in2_history & MASK;

        // info!("P1 {:04b} P2 {:04b}", in1_last_four, in2_last_four);

        if in1_last_four == LEFT_P1 && in2_last_four == LEFT_P2 {
            info!("Rot left");
        } else if in1_last_four == RIGHT_P1 && in2_last_four == RIGHT_P2 {
            info!("Rot right");
        }
    }
}
