#![allow(dead_code)]

use register::{PaConfig, Register, IRQ};
use rppal::spi::Spi;
use rppal::gpio::OutputPin;
use spin_sleep;
use anyhow::{Result,anyhow};
use std::time::Duration;
use bit_field::BitField;


use crate::register;



// 

// const LORA_CS_PIN: u8 = 7;
// const LORA_RESET_PIN: u8 = 25;
const FREQUENCY: i64 = 433;
const VERSION_CHECK: u8 = 0x12;
const TX_CHUNK_SIZE: usize = 255;



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
pub struct LoRa{
    spi: Spi,
    cs: OutputPin,
    reset: OutputPin,
    frequency: i64,
    explicit_header: bool,
    mode: RadioMode, // Assuming RadioMode is defined elsewhere
}

impl LoRa {
    pub fn new(spi: Spi, cs: OutputPin, reset: OutputPin) -> Result<Self> {
        let mut lora = LoRa {
            spi,
            cs,
            reset,
            frequency: FREQUENCY,
            explicit_header: false,
            mode: RadioMode::Sleep,
        };

        lora.reset.set_low();
        spin_sleep::sleep(Duration::from_millis(10));
        lora.reset.set_high();
        spin_sleep::sleep(Duration::from_millis(10));

        let version = lora.read_register(Register::RegVersion.addr())?;

        if version == VERSION_CHECK {
            lora.set_mode(RadioMode::Sleep)?;
            // set to lora mode? - this is done constantly in the set_mode function and is not needed here.
                
            //setup 256 byte fifo.
            lora.write_register(Register::RegFifoTxBaseAddr.addr(), 0)?;
            lora.write_register(Register::RegFifoRxBaseAddr.addr(), 0)?;
            // lora.set_mode(RadioMode::Stdby)?;

            lora.set_frequency(FREQUENCY)?;
            lora.set_preamble_length(8)?;
             
            lora.set_signal_bandwidth(125_000)?;
            lora.set_coding_rate_4(5)?;
            lora.set_spreading_factor(7)?;

            lora.set_crc(true)?;
            lora.set_tx_power(20, 1)?;

            let lna = lora.read_register(Register::RegLna.addr())?;
            lora.write_register(Register::RegLna.addr(), lna | 0x03)?;
            lora.write_register(Register::RegModemConfig3.addr(), 0x04)?;

            lora.set_mode(RadioMode::Stdby)?;
            lora.cs.set_high();
            Ok(lora)
        }else{
            Err(anyhow!("Version mismatch."))
        }
    }

    pub fn read_register(&mut self, reg: u8) -> Result<u8> {
        self.cs.set_low();
        // Prepare the write buffer with the register address, ensuring the MSB is 0 for a read operation
        let write_buffer = [reg & 0x7f, 0];
        // Prepare an empty read buffer to receive data
        let mut read_buffer = [0, 0]; // Same size as write_buffer to ensure full duplex
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

        // Set the default explicit always.. I dont ever change it. This removes the need for this if statement over and over.
        // if self.explicit_header {
        //     self.set_explicit_header_mode()?;
        // } else {
        //     self.set_implicit_header_mode()?;
        // }
        self.write_register(
            Register::RegOpMode.addr(),
            RadioMode::LongRangeMode.addr() | mode.addr(),
        )?;

        self.mode = mode;
        Ok(())
    }

    /// Sets the transmit power and pin. Levels can range from 0-14 when the output
    /// pin = 0(RFO), and form 0-20 when output pin = 1(PaBoost). Power is in dB.
    /// Default value is `17`.
    pub fn set_tx_power(
        &mut self,
        mut level: i32,
        output_pin: u8,
    ) -> Result<()> {
        if PaConfig::PaOutputRfoPin.addr() == output_pin {
            // RFO
            if level < 0 {
                level = 0;
            } else if level > 14 {
                level = 14;
            }
            self.write_register(Register::RegPaConfig.addr(), (0x70 | level) as u8)
        } else {
            // PA BOOST
            if level > 17 {
                if level > 20 {
                    level = 20;
                }
                // subtract 3 from level, so 18 - 20 maps to 15 - 17
                level -= 3;

                // High Power +20 dBm Operation (Semtech SX1276/77/78/79 5.4.3.)
                self.write_register(Register::RegPaDac.addr(), 0x87)?;
                self.set_ocp(140)?;
            } else {
                if level < 2 {
                    level = 2;
                }
                //Default value PA_HF/LF or +17dBm
                self.write_register(Register::RegPaDac.addr(), 0x84)?;
                self.set_ocp(100)?;
            }
            level -= 2;
            self.write_register(
                Register::RegPaConfig.addr(),
                PaConfig::PaBoost.addr() | level as u8,
            )
        }
    }


    /// Sets the radio to use an explicit header. Default state is `ON`.
    fn set_explicit_header_mode(&mut self) -> Result<()> {
        let reg_modem_config_1 = self.read_register(Register::RegModemConfig1.addr())?;
        self.write_register(Register::RegModemConfig1.addr(), reg_modem_config_1 & 0xfe)?;
        self.explicit_header = true;
        Ok(())
    }

    /// Sets the radio to use an implicit header. Default state is `OFF`.
    /// 
    fn set_implicit_header_mode(&mut self) -> Result<()> {
        let reg_modem_config_1 = self.read_register(Register::RegModemConfig1.addr())?;
        println!("Implicit header: config: {:08b}", reg_modem_config_1);
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

    pub fn tx_done(&mut self) -> Result<bool>{
        let res = self.read_register(Register::RegIrqFlags.addr())?;
        println!("{:08b}", res);
        match (res & 0x8) >> 3 {
            0 => Ok(false),
            _ => Ok(true),
        }
    }

    // Sets the frequency of the radio. Values are in megahertz.
    /// I.E. 915 MHz must be used for North America. Check regulation for your area.
    pub fn set_frequency(&mut self, freq: i64) -> Result<()> {
        self.frequency = freq;
        // calculate register values
        let base = 1;
        let frf = (freq * (base << 19)) / 32;
        // write registers
        self.write_register(
            Register::RegFrfMsb.addr(),
            ((frf & 0x00FF_0000) >> 16) as u8,
        )?;
        self.write_register(Register::RegFrfMid.addr(), ((frf & 0x0000_FF00) >> 8) as u8)?;
        self.write_register(Register::RegFrfLsb.addr(), (frf & 0x0000_00FF) as u8)
    }

    /// Sets the over current protection on the radio(mA).
    pub fn set_ocp(&mut self, ma: u8) -> Result<()> {
        let mut ocp_trim: u8 = 27;

        if ma <= 120 {
            ocp_trim = (ma - 45) / 5;
        } else if ma <= 240 {
            ocp_trim = (ma + 30) / 10;
        }
        self.write_register(Register::RegOcp.addr(), 0x20 | (0x1F & ocp_trim))
    }

    /// Returns the signal bandwidth of the radio.
    pub fn get_signal_bandwidth(&mut self) -> Result<i64> {
        let bw = self.read_register(Register::RegModemConfig1.addr())? >> 4;
        let bw = match bw {
            0 => 7_800,
            1 => 10_400,
            2 => 15_600,
            3 => 20_800,
            4 => 31_250,
            5 => 41_700,
            6 => 62_500,
            7 => 125_000,
            8 => 250_000,
            9 => 500_000,
            _ => -1,
        };
        Ok(bw)
    }

    /// Returns the spreading factor of the radio.
    pub fn get_spreading_factor(&mut self) -> Result<u8> {
        Ok(self.read_register(Register::RegModemConfig2.addr())? >> 4)
    }

    fn set_ldo_flag(&mut self) -> Result<()> {
        let sw = self.get_signal_bandwidth()?;
        // Section 4.1.1.5
        let symbol_duration = 1000 / (sw / ((1 as i64) << self.get_spreading_factor()?));

        // Section 4.1.1.6
        let ldo_on = symbol_duration > 16;

        let mut config_3 = self.read_register(Register::RegModemConfig3.addr())?;
        config_3.set_bit(3, ldo_on);
        self.write_register(Register::RegModemConfig3.addr(), config_3)
    }

        /// Sets the spreading factor of the radio. Supported values are between 6 and 12.
    /// If a spreading factor of 6 is set, implicit header mode must be used to transmit
    /// and receive packets. Default value is `7`.
    pub fn set_spreading_factor(
        &mut self,
        mut sf: u8,
    ) -> Result<()> {
        if sf < 6 {
            sf = 6;
        } else if sf > 12 {
            sf = 12;
        }

        if sf == 6 {
            self.write_register(Register::RegDetectionOptimize.addr(), 0xc5)?;
            self.write_register(Register::RegDetectionThreshold.addr(), 0x0c)?;
        } else {
            self.write_register(Register::RegDetectionOptimize.addr(), 0xc3)?;
            self.write_register(Register::RegDetectionThreshold.addr(), 0x0a)?;
        }
        let modem_config_2 = self.read_register(Register::RegModemConfig2.addr())?;
        self.write_register(
            Register::RegModemConfig2.addr(),
            (modem_config_2 & 0x0f) | ((sf << 4) & 0xf0),
        )?;
        self.set_ldo_flag()?;
        Ok(())
    }

    /// Transmits up to 255 bytes of data. To avoid the use of an allocator, this takes a fixed 255 u8
    /// array and a payload size and returns the number of bytes sent if successful.
    pub fn transmit_payload_busy(&mut self,buffer: [u8; 255],payload_size: usize,) -> Result<usize> {
        if self.transmitting()? {
            Err(anyhow!("ALREADY TX"))
        } else {
            self.set_mode(RadioMode::Stdby)?;
            // if self.explicit_header {
            //     self.set_explicit_header_mode()?;
            // } else {
            //     self.set_implicit_header_mode()?;
            // }

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

        /// Sets the preamble length of the radio. Values are between 6 and 65535.
    /// Default value is `8`.
    pub fn set_preamble_length(
        &mut self,
        length: i64,
    ) -> Result<()> {
        self.write_register(Register::RegPreambleMsb.addr(), (length >> 8) as u8)?;
        self.write_register(Register::RegPreambleLsb.addr(), length as u8)
    }

    /// Enables are disables the radio's CRC check. Default value is `false`.
    pub fn set_crc(&mut self, value: bool) -> Result<()> {
        let modem_config_2 = self.read_register(Register::RegModemConfig2.addr())?;
        if value {
            self.write_register(Register::RegModemConfig2.addr(), modem_config_2 | 0x04)
        } else {
            self.write_register(Register::RegModemConfig2.addr(), modem_config_2 & 0xfb)
        }
    }

    /// Inverts the radio's IQ signals. Default value is `false`.
    pub fn set_invert_iq(&mut self, value: bool) -> Result<()> {
        if value {
            self.write_register(Register::RegInvertiq.addr(), 0x66)?;
            self.write_register(Register::RegInvertiq2.addr(), 0x19)
        } else {
            self.write_register(Register::RegInvertiq.addr(), 0x27)?;
            self.write_register(Register::RegInvertiq2.addr(), 0x1d)
        }
    }

    /// Sets the signal bandwidth of the radio. Supported values are: `7800 Hz`, `10400 Hz`,
    /// `15600 Hz`, `20800 Hz`, `31250 Hz`,`41700 Hz` ,`62500 Hz`,`125000 Hz` and `250000 Hz`
    /// Default value is `125000 Hz`
    /// See p. 4 of SX1276_77_8_ErrataNote_1.1_STD.pdf for Errata implemetation
    pub fn set_signal_bandwidth(
        &mut self,
        sbw: i64,
    ) -> Result<()> {
        let bw: i64 = match sbw {
            7_800 => 0,
            10_400 => 1,
            15_600 => 2,
            20_800 => 3,
            31_250 => 4,
            41_700 => 5,
            62_500 => 6,
            125_000 => 7,
            250_000 => 8,
            _ => 9,
        };

        if bw == 9 {
            if self.frequency < 525 {
                self.write_register(Register::RegHighBWOptimize1.addr(), 0x02)?;
                self.write_register(Register::RegHighBWOptimize2.addr(), 0x7f)?;
            } else {
                self.write_register(Register::RegHighBWOptimize1.addr(), 0x02)?;
                self.write_register(Register::RegHighBWOptimize2.addr(), 0x64)?;
            }
        } else {
            self.write_register(Register::RegHighBWOptimize1.addr(), 0x03)?;
            self.write_register(Register::RegHighBWOptimize2.addr(), 0x65)?;
        }

        let modem_config_1 = self.read_register(Register::RegModemConfig1.addr())?;
        self.write_register(
            Register::RegModemConfig1.addr(),
            (modem_config_1 & 0x0f) | ((bw << 4) as u8),
        )?;
        self.set_ldo_flag()?;
        Ok(())
    }

    /// Sets the coding rate of the radio with the numerator fixed at 4. Supported values
    /// are between `5` and `8`, these correspond to coding rates of `4/5` and `4/8`.
    /// Default value is `5`.
    pub fn set_coding_rate_4(
        &mut self,
        mut denominator: u8,
    ) -> Result<()> {
        if denominator < 5 {
            denominator = 5;
        } else if denominator > 8 {
            denominator = 8;
        }
        let cr = denominator - 4;
        let modem_config_1 = self.read_register(Register::RegModemConfig1.addr())?;
        self.write_register(
            Register::RegModemConfig1.addr(),
            (modem_config_1 & 0xf1) | (cr << 1),
        )
    }


    pub fn transmit_payload(&mut self,payload: &[u8],) -> Result<()> {
        if self.transmitting()? {
            Err(anyhow!("Transmitting"))
        } else {
            self.set_mode(RadioMode::Stdby)?;
            // if self.explicit_header {
            //     self.set_explicit_header_mode()?;
            // } else {
            //     self.set_implicit_header_mode()?;
            // }

            self.write_register(Register::RegIrqFlags.addr(), 0)?;
            self.write_register(Register::RegFifoAddrPtr.addr(), 0)?;
            self.write_register(Register::RegPayloadLength.addr(), 0)?;
            for &byte in payload.iter().take(255) {
                self.write_register(Register::RegFifo.addr(), byte)?;
            }
            self.write_register(
                Register::RegPayloadLength.addr(),
                payload.len().min(255) as u8,
            )?;
            self.set_mode(RadioMode::Tx)?;
            Ok(())
        }
    }


    /// Returns size of a packet read into FIFO. This should only be calle if there is a new packet
    /// ready to be read.
    pub fn get_ready_packet_size(&mut self) -> Result<u8> {
        self.read_register(Register::RegRxNbBytes.addr())
    }

    /// Returns the contents of the fifo as a fixed 255 u8 array. This should only be called if there is a
    /// new packet ready to be read.
    pub fn read_packet(&mut self) -> Result<[u8; 255]> {
        let mut buffer = [0 as u8; 255];
        self.clear_irq()?;
        let size = self.get_ready_packet_size()?;
        let fifo_addr = self.read_register(Register::RegFifoRxCurrentAddr.addr())?;
        self.write_register(Register::RegFifoAddrPtr.addr(), fifo_addr)?;
        for i in 0..size {
            let byte = self.read_register(Register::RegFifo.addr())?;
            buffer[i as usize] = byte;
        }
        self.write_register(Register::RegFifoAddrPtr.addr(), 0)?;
        Ok(buffer)
    }

    pub fn tx_bulk(&mut self, data: &[u8]) {

        // interrupting on tx done.
        self.write_register(Register::RegDioMapping1.addr(), 0x1).unwrap();

        for chunk in data.chunks(TX_CHUNK_SIZE){
            let mut buffer = [0u8; TX_CHUNK_SIZE]; // Initialize a buffer with zeros

            // Copy the chunk into the buffer. The chunk can be smaller than the buffer for the last piece of data.
            let chunk_len = chunk.len();
            buffer[..chunk_len].copy_from_slice(chunk);


            if chunk_len < TX_CHUNK_SIZE {
                self.transmit_payload_busy(buffer, chunk_len).unwrap();
            } else {
                self.transmit_payload_busy(buffer, 255).unwrap(); // Transmit the full buffer
            }
        }

    }

}
