# Simple Volume Knob

A simple Pi Pico W based device that lets you control the volume of your pc
using a knob.

## Building it yourself
You can build this device yourself.<br>
You need:
- Raspberry Pi Pico W
- Rotary encoder
- Wires to connect them

The pinout is as follows:<br>
The rotary encoder rotation pins should be connected to GP16 and GP17.<br>
The button pin should be connected to GP18.<br>
Check official [pinout](https://pip-assets.raspberrypi.com/categories/686-raspberry-pi-pico-w/documents/RP-008315-DS-1-PicoW-A4-Pinout.pdf?disposition=inline) diagram to locate them, and don't forget to connect the ground :)

## Flashing
If you have a debug probe for pico you need to setup [probe-rs](https://probe.rs/) and run `cargo run -r`.<br>
Otherwise see Embassy [docs](https://embassy.dev/book/#_how_to_deploy_to_rp2040_or_rp235x_without_a_debugging_probe) on how to run it without debug probe.

