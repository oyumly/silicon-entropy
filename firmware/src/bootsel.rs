use core::ptr::{read_volatile, write_volatile};

const IO_QSPI_GPIO_QSPI_SS_CTRL: *mut u32 = 0x4001_800C as *mut u32;
const SIO_GPIO_HI_IN: *const u32 = 0xD000_0008 as *const u32;

// Must run from SRAM because QSPI_SS is temporarily disabled
#[inline(never)]
#[link_section = ".data.ram_func"]
pub fn is_bootsel_pressed() -> bool {
    cortex_m::interrupt::free(|_| unsafe {
        let prev = read_volatile(IO_QSPI_GPIO_QSPI_SS_CTRL);
        write_volatile(IO_QSPI_GPIO_QSPI_SS_CTRL, (prev & !(0x3 << 12)) | (0x2 << 12)); // OEOVER = 2

        for _ in 0..32 {
            cortex_m::asm::nop();
        }

        let pressed = (read_volatile(SIO_GPIO_HI_IN) & (1 << 1)) == 0;

        write_volatile(IO_QSPI_GPIO_QSPI_SS_CTRL, prev);
        cortex_m::asm::dsb();
        cortex_m::asm::isb();

        pressed
    })
}
