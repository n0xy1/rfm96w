#![allow(dead_code)]

use register::{Register, IRQ};
use rppal::spi::{Bus, Mode, SlaveSelect, Spi};
use rppal::gpio::{Gpio, OutputPin};
use spin_sleep;
use anyhow::{Result,anyhow};
use std::time::Duration;
use bit_field::BitField;
mod register;

const LORA_CS_PIN: u8 = 7;
const LORA_RESET_PIN: u8 = 25;
const FREQUENCY: i64 = 433;

/// Modes of the radio and their corresponding register values.
#[derive(Clone, Copy)]
pub enum RadioMode {
    LongRangeMode = 0x80,
    Sleep = 0x00,
    Stdby = 0x01,
    FsTx = 0x02,
    Tx = 0x03,
    RxContinuous = 0x05,
    RxSingle = 0x06,
}

impl RadioMode {
    /// Returns the address of the mode.
    pub fn addr(self) -> u8 {
        self as u8
    }
}


// Add a lifetime parameter `'a` and a generic type parameter `D` bounded by the `DelayMs<u8>` trait
struct LoRa{
    spi: Spi,
    cs: OutputPin,
    reset: OutputPin,
    frequency: i64,
    explicit_header: bool,
    mode: RadioMode, // Assuming RadioMode is defined elsewhere
}

impl LoRa {
    // The `new` function takes `delay` as a mutable reference to a `D` that implements `DelayMs<u8>`
    pub fn new(spi: Spi, cs: OutputPin, reset: OutputPin) -> Result<Self> {
        let mut lora = LoRa {
            spi,
            cs,
            reset,
            frequency: FREQUENCY,
            explicit_header: false,
            mode: RadioMode::Sleep, // Assuming you have a `RadioMode::Sleep` variant
        };

        // Initialize or reset the LoRa module as needed here
        // For example, you might want to pull the reset pin high, then low, then high again to reset the module.
        lora.reset.set_low();
        spin_sleep::sleep(Duration::from_millis(10));
        lora.reset.set_high();
        spin_sleep::sleep(Duration::from_millis(10));
        lora.reset.set_low();

        // Perform any necessary SPI or other initialization here

        Ok(lora)
    }

    pub fn read_register(&mut self, reg: u8) -> Result<u8> {
        self.cs.set_low();
        // Prepare the write buffer with the register address, ensuring the MSB is 0 for a read operation
        let write_buffer = [reg & 0x7f, 0]; // Second byte is dummy because SPI is full duplex
        // Prepare an empty read buffer to receive data
        let mut read_buffer = [0u8; 2]; // Same size as write_buffer to ensure full duplex
        // Perform the SPI transfer
        self.spi.transfer(&mut read_buffer, &write_buffer).map_err(anyhow::Error::new)?;
        self.cs.set_high();
        

        // Return the data, which is now in read_buffer[1], as the second byte of the buffer is where the actual data is read into
        Ok(read_buffer[1])
    }

    fn write_register(&mut self, reg: u8, byte: u8,) -> Result<()> {
        self.cs.set_low();

        let buffer = [reg | 0x80, byte];
        self.spi.write(&buffer).map_err(anyhow::Error::msg)?;
        self.cs.set_high();
        Ok(())
    }
    
    /// Sets the state of the radio. Default mode after initiation is `Standby`.
    pub fn set_mode(&mut self, mode: RadioMode) -> Result<()> {
        if self.explicit_header {
            self.set_explicit_header_mode()?;
        } else {
            self.set_implicit_header_mode()?;
        }
        self.write_register(
            Register::RegOpMode.addr(),
            RadioMode::LongRangeMode.addr() | mode.addr(),
        )?;

        self.mode = mode;
        Ok(())
    }

    /// Sets the radio to use an explicit header. Default state is `ON`.
    fn set_explicit_header_mode(&mut self) -> Result<()> {
        let reg_modem_config_1 = self.read_register(Register::RegModemConfig1.addr())?;
        self.write_register(Register::RegModemConfig1.addr(), reg_modem_config_1 & 0xfe)?;
        self.explicit_header = true;
        Ok(())
    }

    /// Sets the radio to use an implicit header. Default state is `OFF`.
    fn set_implicit_header_mode(&mut self) -> Result<()> {
        let reg_modem_config_1 = self.read_register(Register::RegModemConfig1.addr())?;
        self.write_register(Register::RegModemConfig1.addr(), reg_modem_config_1 & 0x01)?;
        self.explicit_header = false;
        Ok(())
    }

    /// Blocks the current thread, returning the size of a packet if one is received or an error is the
    /// task timed out. The timeout can be supplied with None to make it poll indefinitely or
    /// with `Some(timeout_in_mill_seconds)`
    /// 
    pub fn poll_irq(&mut self,timeout_ms: Option<i32>) -> Result<usize> {
        self.set_mode(RadioMode::RxContinuous)?;
        match timeout_ms {
            Some(value) => {
                let mut count = 0;
                let packet_ready = loop {
                    let packet_ready = self.read_register(Register::RegIrqFlags.addr())?.get_bit(6);
                    if count >= value || packet_ready {
                        break packet_ready;
                    }
                    count += 1;
                    spin_sleep::sleep(Duration::from_millis(1));
                };
                if packet_ready {
                    self.clear_irq()?;
                    Ok(self.read_register(Register::RegRxNbBytes.addr())? as usize)
                } else {
                    Err(anyhow!("poll failed"))
                }
            }
            None => {
                while !self.read_register(Register::RegIrqFlags.addr())?.get_bit(6) {
                    spin_sleep::sleep(Duration::from_millis(100));
                }
                self.clear_irq()?;
                Ok(self.read_register(Register::RegRxNbBytes.addr())? as usize)
            }
        }
    }
    
     /// Clears the radio's IRQ registers.
     pub fn clear_irq(&mut self) -> Result<()> {
        let irq_flags = self.read_register(Register::RegIrqFlags.addr())?;
        self.write_register(Register::RegIrqFlags.addr(), irq_flags)
    }

    // /// Returns true if the radio is currently transmitting a packet.
    pub fn transmitting(&mut self) -> Result<bool> {
        let op_mode = self.read_register(Register::RegOpMode.addr())?;
        if (op_mode & RadioMode::Tx.addr()) == RadioMode::Tx.addr()
            || (op_mode & RadioMode::FsTx.addr()) == RadioMode::FsTx.addr()
        {
            Ok(true)
        } else {
            if (self.read_register(Register::RegIrqFlags.addr())? & IRQ::IrqTxDoneMask.addr()) == 1
            {
                self.write_register(Register::RegIrqFlags.addr(), IRQ::IrqTxDoneMask.addr())?;
            }
            Ok(false)
        }
    }


    /// Transmits up to 255 bytes of data. To avoid the use of an allocator, this takes a fixed 255 u8
    /// array and a payload size and returns the number of bytes sent if successful.
    pub fn transmit_payload_busy(&mut self,buffer: [u8; 255],payload_size: usize,) -> Result<usize> {
        if self.transmitting()? {
            Err(anyhow!("ALREADY TX"))
        } else {
            self.set_mode(RadioMode::Stdby)?;
            if self.explicit_header {
                self.set_explicit_header_mode()?;
            } else {
                self.set_implicit_header_mode()?;
            }

            self.write_register(Register::RegIrqFlags.addr(), 0)?;
            self.write_register(Register::RegFifoAddrPtr.addr(), 0)?;
            self.write_register(Register::RegPayloadLength.addr(), 0)?;
            for byte in buffer.iter().take(payload_size) {
                self.write_register(Register::RegFifo.addr(), *byte)?;
            }
            self.write_register(Register::RegPayloadLength.addr(), payload_size as u8)?;
            self.set_mode(RadioMode::Tx)?;
            while self.transmitting()? {}
            Ok(payload_size)
        }
    }

}

fn main() -> Result<()> {
    let spi = Spi::new(Bus::Spi0, SlaveSelect::Ss0, 20_000, Mode::Mode0)?;
    let cs_pin = Gpio::new()?.get(LORA_CS_PIN)?.into_output();
    let reset_pin = Gpio::new()?.get(LORA_RESET_PIN)?.into_output();
    let mut radio = LoRa::new(spi, cs_pin, reset_pin)?;

    let res = radio.transmitting()?;
    println!("{:?}", res);
    Ok(())
}