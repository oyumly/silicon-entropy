use core::ptr::{read_volatile, write_volatile};

const ROSC_BASE: usize = 0x4006_0000;
const ROSC_CTRL: *mut u32 = ROSC_BASE as *mut u32;
const ROSC_DIV: *mut u32 = (ROSC_BASE + 0x10) as *mut u32;
const ROSC_RANDOMBIT: *const u32 = (ROSC_BASE + 0x1C) as *const u32;

const ADC_BASE: usize = 0x4004_C000;
const ADC_CS: *mut u32 = ADC_BASE as *mut u32;
const ADC_RESULT: *const u32 = (ADC_BASE + 0x04) as *const u32;

const RESETS_RESET: *mut u32 = 0x4000_C000 as *mut u32;
const RESETS_RESET_DONE: *const u32 = 0x4000_C008 as *const u32;
const RESET_ADC_BIT: u32 = 1 << 0;

pub struct EntropySource;

impl EntropySource {
    pub fn init() -> Self {
        unsafe {
            // Ensure ROSC is enabled (ENABLE = 0xFAB in bits 23:12, default freq range)
            let ctrl = read_volatile(ROSC_CTRL);
            if (ctrl >> 12) & 0xFFF != 0xFAB {
                write_volatile(ROSC_CTRL, (0xFAB << 12) | 0xAA0);
            }
            // Set ROSC divider to 1 for maximum free-running frequency (0xAA0 + 1)
            write_volatile(ROSC_DIV, 0xAA1);

            // De-assert reset on ADC
            let rst = read_volatile(RESETS_RESET);
            write_volatile(RESETS_RESET, rst & !RESET_ADC_BIT);
            while (read_volatile(RESETS_RESET_DONE) & RESET_ADC_BIT) == 0 {}

            // Enable ADC + internal temperature sensor bias (TS_EN = bit 1, EN = bit 0)
            // AINSEL = 4 (temperature sensor, bits 14:12)
            write_volatile(ADC_CS, (4 << 12) | (1 << 1) | 1);
            // Wait until ADC is ready (READY = bit 8)
            while (read_volatile(ADC_CS) & (1 << 8)) == 0 {}
        }
        Self
    }

    #[inline(always)]
    pub fn sample_rosc_bit(&self) -> u8 {
        // ~380 ns at 125 MHz so ROSC completes multiple cycles between reads
        for _ in 0..48 {
            cortex_m::asm::nop();
        }
        unsafe { (read_volatile(ROSC_RANDOMBIT) & 1) as u8 }
    }

    #[inline(always)]
    fn sample_adc_lsb(&self) -> u8 {
        unsafe {
            let cs = read_volatile(ADC_CS);
            write_volatile(ADC_CS, cs | (1 << 2)); // START_ONCE
            while (read_volatile(ADC_CS) & (1 << 8)) == 0 {}
            (read_volatile(ADC_RESULT) & 1) as u8
        }
    }

    pub fn next_raw_byte(&self) -> u8 {
        let mut byte: u8 = 0;
        for i in 0..8 {
            let bit = self.sample_rosc_bit();
            byte |= bit << i;
        }
        byte ^ self.sample_adc_lsb()
    }
}
