#![no_std]
#![no_main]

mod bootsel;
mod entropy;
mod health;
mod whitener;

use core::fmt::Write as _;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use embedded_hal::digital::OutputPin;
use panic_halt as _;
use rp2040_hal as hal;
use rp2040_hal::multicore::{Multicore, Stack};
use rp2040_hal::pac;
use usb_device::class_prelude::*;
use usb_device::prelude::*;
use usbd_serial::SerialPort;

use entropy::EntropySource;
use health::HealthMonitor;
use whitener::Whitener;

/// Generic 03h boot2 works reliably across both original Winbond and clone QSPI flash chips.
#[link_section = ".boot2"]
#[used]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_GENERIC_03H;

const XTAL_FREQ_HZ: u32 = 12_000_000;
const RING_SIZE: usize = 4096;

static mut CORE1_STACK: Stack<4096> = Stack::new();

// Lock-free SPSC ring buffer (Core 1 produces, Core 0 consumes)
static mut RING_BUF: [u8; RING_SIZE] = [0; RING_SIZE];
static RING_HEAD: AtomicUsize = AtomicUsize::new(0);
static RING_TAIL: AtomicUsize = AtomicUsize::new(0);

// Shared atomic telemetry from Core 1 -> Core 0
static STAT_HEALTHY: AtomicBool = AtomicBool::new(true);
static STAT_TOTAL_BYTES: AtomicU32 = AtomicU32::new(0);
static STAT_RCT_FAILS: AtomicU32 = AtomicU32::new(0);
static STAT_APT_FAILS: AtomicU32 = AtomicU32::new(0);
static STAT_ONES_1K: AtomicU32 = AtomicU32::new(512);

// Safe XIP handshake so Core 1 stays in SRAM while Core 0 reads BOOTSEL
static XIP_PAUSE_REQ: AtomicBool = AtomicBool::new(false);
static XIP_CORE1_PAUSED: AtomicBool = AtomicBool::new(false);

#[inline(never)]
#[link_section = ".data.ram_func"]
fn wait_xip_resume() {
    XIP_CORE1_PAUSED.store(true, Ordering::Release);
    while XIP_PAUSE_REQ.load(Ordering::Acquire) {
        cortex_m::asm::nop();
    }
    XIP_CORE1_PAUSED.store(false, Ordering::Release);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StreamMode {
    Whitened,
    Raw,
}

/// Core 1 entry point: continuous physical entropy sampling + NIST SP 800-90B health tests.
fn core1_entropy_task() -> ! {
    let source = EntropySource::init();
    let mut monitor = HealthMonitor::new();

    loop {
        if XIP_PAUSE_REQ.load(Ordering::Acquire) {
            wait_xip_resume();
        }
        let head = RING_HEAD.load(Ordering::Relaxed);
        let next_head = (head + 1) & (RING_SIZE - 1);
        let tail = RING_TAIL.load(Ordering::Acquire);

        if next_head != tail {
            let raw = source.next_raw_byte();
            let healthy = monitor.feed_byte(raw);

            // Update telemetry periodically
            if (monitor.stats.total_bytes & 0xFF) == 0 || !healthy {
                STAT_HEALTHY.store(healthy, Ordering::Relaxed);
                STAT_TOTAL_BYTES.store(monitor.stats.total_bytes, Ordering::Relaxed);
                STAT_RCT_FAILS.store(monitor.stats.rct_failures, Ordering::Relaxed);
                STAT_APT_FAILS.store(monitor.stats.apt_failures, Ordering::Relaxed);
                STAT_ONES_1K.store(monitor.stats.last_ones_per_1k, Ordering::Relaxed);
            }

            if healthy {
                unsafe {
                    RING_BUF[head] = raw;
                }
                RING_HEAD.store(next_head, Ordering::Release);
            }
        } else {
            cortex_m::asm::nop();
        }
    }
}

/// Pop up to `dst.len()` bytes from the SPSC ring buffer.
fn pop_entropy(dst: &mut [u8]) -> usize {
    let mut count = 0;
    let mut tail = RING_TAIL.load(Ordering::Relaxed);
    let head = RING_HEAD.load(Ordering::Acquire);

    while count < dst.len() && tail != head {
        dst[count] = unsafe { RING_BUF[tail] };
        tail = (tail + 1) & (RING_SIZE - 1);
        count += 1;
    }

    if count > 0 {
        RING_TAIL.store(tail, Ordering::Release);
    }
    count
}

struct SmallString {
    buf: [u8; 160],
    len: usize,
}

impl SmallString {
    fn new() -> Self {
        Self {
            buf: [0; 160],
            len: 0,
        }
    }
    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl core::fmt::Write for SmallString {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let avail = self.buf.len().saturating_sub(self.len);
        let n = bytes.len().min(avail);
        self.buf[self.len..self.len + n].copy_from_slice(&bytes[..n]);
        self.len += n;
        Ok(())
    }
}

#[hal::entry]
fn main() -> ! {
    let mut pac = pac::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);

    let clocks = hal::clocks::init_clocks_and_plls(
        XTAL_FREQ_HZ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    let mut sio = hal::Sio::new(pac.SIO);
    let pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );
    let mut led = pins.gpio25.into_push_pull_output();

    // Launch Core 1 for continuous entropy collection & NIST health monitoring
    let mut mc = Multicore::new(&mut pac.PSM, &mut pac.PPB, &mut sio.fifo);
    let cores = mc.cores();
    let core1 = &mut cores[1];
    let _ = core1.spawn(unsafe { &mut CORE1_STACK.mem }, || core1_entropy_task());

    // Initialize USB CDC
    let usb_bus = UsbBusAllocator::new(hal::usb::UsbBus::new(
        pac.USBCTRL_REGS,
        pac.USBCTRL_DPRAM,
        clocks.usb_clock,
        true,
        &mut pac.RESETS,
    ));

    let mut serial = SerialPort::new(&usb_bus);
    let mut usb_dev = UsbDeviceBuilder::new(&usb_bus, UsbVidPid(0x2E8A, 0x000A))
        .strings(&[StringDescriptors::default()
            .manufacturer("silicon-entropy")
            .product("RP2040 Hardware TRNG")
            .serial_number("SE-0001")])
        .unwrap()
        .device_class(usbd_serial::USB_CLASS_CDC)
        .build();

    let mut whitener = Whitener::new();
    let mut mode = StreamMode::Whitened;
    let mut streaming = true;

    let mut raw_acc = [0u8; 64];
    let mut raw_acc_len: usize = 0;
    let mut tx_buf = [0u8; 64];
    let mut tx_len: usize = 0;
    let mut tx_pos: usize = 0;

    let mut tick: u32 = 0;
    let mut prev_btn = false;

    loop {
        usb_dev.poll(&mut [&mut serial]);

        // Poll BOOTSEL button safely (~every 16384 iterations)
        tick = tick.wrapping_add(1);
        if (tick & 0x3FFF) == 0 {
            XIP_PAUSE_REQ.store(true, Ordering::Release);
            while !XIP_CORE1_PAUSED.load(Ordering::Acquire) {
                cortex_m::asm::nop();
            }
            let btn = bootsel::is_bootsel_pressed();
            XIP_PAUSE_REQ.store(false, Ordering::Release);
            if btn && !prev_btn {
                mode = match mode {
                    StreamMode::Whitened => StreamMode::Raw,
                    StreamMode::Raw => StreamMode::Whitened,
                };
            }
            prev_btn = btn;

            // LED status indicator
            let healthy = STAT_HEALTHY.load(Ordering::Relaxed);
            if !healthy {
                // Fast alarm blink on NIST failure
                if (tick & 0x2000) != 0 {
                    let _ = led.set_high();
                } else {
                    let _ = led.set_low();
                }
            } else if mode == StreamMode::Whitened {
                let _ = led.set_high();
            } else {
                // Slow blink in Raw mode
                if (tick & 0x10000) != 0 {
                    let _ = led.set_high();
                } else {
                    let _ = led.set_low();
                }
            }
        }

        // Handle incoming USB commands
        let mut cmd_buf = [0u8; 16];
        if let Ok(n) = serial.read(&mut cmd_buf) {
            for &b in &cmd_buf[..n] {
                match b {
                    b'w' | b'W' => {
                        mode = StreamMode::Whitened;
                        tx_len = 0;
                        tx_pos = 0;
                    }
                    b'r' | b'R' => {
                        mode = StreamMode::Raw;
                        tx_len = 0;
                        tx_pos = 0;
                    }
                    b'p' | b'P' => streaming = !streaming,
                    b'1' => {
                        streaming = true;
                        tx_len = 0;
                        tx_pos = 0;
                    }
                    b'0' => streaming = false,
                    b'u' | b'U' => {
                        hal::rom_data::reset_to_usb_boot(0, 0);
                    }
                    b's' | b'S' => {
                        let mut msg = SmallString::new();
                        let mode_str = match mode {
                            StreamMode::Whitened => "whitened",
                            StreamMode::Raw => "raw",
                        };
                        let _ = writeln!(
                            msg,
                            "{{\"mode\":\"{}\",\"healthy\":{},\"bytes\":{},\"rct_fail\":{},\"apt_fail\":{},\"ones_1k\":{}}}",
                            mode_str,
                            STAT_HEALTHY.load(Ordering::Relaxed),
                            STAT_TOTAL_BYTES.load(Ordering::Relaxed),
                            STAT_RCT_FAILS.load(Ordering::Relaxed),
                            STAT_APT_FAILS.load(Ordering::Relaxed),
                            STAT_ONES_1K.load(Ordering::Relaxed),
                        );
                        let _ = serial.write(msg.as_bytes());
                    }
                    _ => {}
                }
            }
        }

        // Stream entropy over USB CDC when healthy & active
        if streaming
            && usb_dev.state() == UsbDeviceState::Configured
            && STAT_HEALTHY.load(Ordering::Relaxed)
        {
            if tx_pos >= tx_len {
                tx_pos = 0;
                tx_len = 0;
                match mode {
                    StreamMode::Raw => {
                        tx_len = pop_entropy(&mut tx_buf);
                    }
                    StreamMode::Whitened => {
                        let need = 64 - raw_acc_len;
                        let got = pop_entropy(&mut raw_acc[raw_acc_len..raw_acc_len + need]);
                        raw_acc_len += got;
                        if raw_acc_len == 64 {
                            let out = whitener.whiten_block(&raw_acc);
                            raw_acc_len = 0;
                            tx_buf[..32].copy_from_slice(&out);
                            tx_len = 32;
                        }
                    }
                }
            }

            if tx_pos < tx_len {
                if let Ok(written) = serial.write(&tx_buf[tx_pos..tx_len]) {
                    tx_pos += written;
                }
            }
        }
    }
}
