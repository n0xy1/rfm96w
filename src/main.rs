// general prog
use anyhow::Result;
// threads
use std::io::{Write, BufReader, BufRead};
use std::process::{Command, Stdio};
use std::thread;
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::time::Duration;
// radio specific stuff.
use rfm96w::LoRa;
use rppal::{gpio::Gpio, spi::{Bus, Mode, SlaveSelect, Spi}};

mod register;
mod rfm96w;

// 
const LORA_CS_PIN: u8 = 7;
const LORA_RESET_PIN: u8 = 25;
const G0_PIN: u8 = 5;
const PYTHON_HEADER: [u8;4] = [255,255,0,0];

fn trim_trailing_zeros(data: &[u8]) -> &[u8] {
    // Find the last non-zero byte in the array. `enumerate()` provides the index alongside the value.
    // `rev()` reverses the iterator, so we search from the end towards the start.
    // `find()` returns the first element satisfying the condition, which is the last non-zero byte due to reversal.
    if let Some((pos, _)) = data.iter().enumerate().rev().find(|&(_, &x)| x != 0) {
        // Slice the array from the start up to the position of the last non-zero byte, inclusive.
        // We add 1 to include the non-zero byte itself in the slice.
        &data[..=pos]
    } else {
        // If all bytes are zero, return an empty slice.
        &[]
    }
}

fn main() -> Result<()> {
    let spi = Spi::new(Bus::Spi0, SlaveSelect::Ss0,5_000_000, Mode::Mode0)?;
    let cs_pin = Gpio::new()?.get(LORA_CS_PIN)?.into_output();
    let reset_pin = Gpio::new()?.get(LORA_RESET_PIN)?.into_output();

    let mut radio = LoRa::new(spi, cs_pin, reset_pin)?;
    let message = "RORA";
    radio.transmit_payload(message.as_bytes())?;

    loop{
        let poll = radio.poll_irq(Some(300));
        match poll {
            Ok(size) => {
                let buffer = radio.read_packet();
                match buffer {
                    Ok(b) => {
                        //rx a buffer!
                        if b[4..8] == *"RORA".as_bytes(){
                            println!{"RX HANDSHAKE!"}
                            break;
                        }
                        println!("RX {} bytes.", size);
                    }
                    Err(_) => {
                        println!("Read packet failed.");
                    },
                }

            },
            Err(_) => println!("timeout"),
        }
    }

    loop{
        // g0 does fck all.
        // if g0_pin.is_high() {
        //     println!("G0 HIGH!");
        // }else{
        //     println!("G0 LOW!");
        // }
        let poll = radio.poll_irq(None);
        match poll {
            Ok(size) => {
                let buffer = radio.read_packet();
                match buffer {
                    Ok(b) => {
                        //rx a buffer!
                        println!("RX {} bytes.", size);
                        // spin_sleep::sleep(Duration::from_millis(10));
                        let mut echo: Vec<u8> = Vec::new();
                        echo.extend_from_slice(&PYTHON_HEADER);
                        echo.extend_from_slice(&b[4..size]);
                        radio.tx_bulk(&echo);
                        println!("TX: {:?}", echo);

                    }
                    Err(_) => {
                        println!("Read packet failed.");
                    },
                }

            },
            Err(_) => {},
        }
    }

    Ok(())
}