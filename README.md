# silicon-entropy

![Rust](https://img.shields.io/badge/Rust-no__std-000000?style=flat-square&logo=rust)
![Target](https://img.shields.io/badge/RP2040-Dual%20Cortex--M0%2B-e84a7f?style=flat-square&logo=raspberrypi)

> *Work in progress — firmware and WebSerial visualizers are being pushed incrementally.*

Dual-core **RP2040 Hardware True Random Number Generator (TRNG)** in bare-metal **Rust** (`#![no_std]`), featuring direct datasheet register access, on-chip **NIST SP 800-90B** health tests, **SHA-256** cryptographic whitening, and zero-install **WebSerial** visualizers over USB-C.

No soldering or external components required — runs on any Raspberry Pi Pico (or RP2040 clone) using only the USB port and the on-board `BOOTSEL` button.

## Architecture

```mermaid
flowchart LR
    subgraph Core1["Core 1 — Physics & Health"]
        ROSC["ROSC Jitter<br/>0x4006001C"] --> MIX["Raw Byte<br/>Collector"]
        ADC["ADC Temp LSB<br/>0x4004C000"] --> MIX
        MIX --> NIST["NIST SP 800-90B<br/>RCT + APT"]
    end

    NIST -->|"Validated bytes"| RING[("Lock-Free SPSC<br/>Ring Buffer")]

    subgraph Core0["Core 0 — Conditioning & I/O"]
        RING --> SHA["SHA-256 Whitener<br/>(2:1 compression)"]
        SHA --> USB["USB-C CDC Stream<br/>(12 Mbps)"]
        BTN["BOOTSEL in SRAM<br/>0x4001800C"] -.->|"Raw / Whitened"| USB
    end
```

## Datasheet Register Map

| Peripheral | Register | Address | Purpose |
| :--- | :--- | :--- | :--- |
| `ROSC` | `RANDOMBIT` | `0x4006001C` | Free-running ring oscillator jitter bit |
| `ADC` | `CS` / `RESULT` | `0x4004C000` | Internal temperature sensor thermal noise LSB |
| `IO_QSPI` | `GPIO_QSPI_SS_CTRL` | `0x4001800C` | Runtime `BOOTSEL` polling from SRAM |
| `SIO` | `GPIO_HI_IN` | `0xD0000008` | QSPI button state read |

## Roadmap

- [x] Hardware architecture & RP2040 datasheet register map
- [ ] Bare-metal Rust firmware (`ROSC` + `ADC`, `NIST SP 800-90B`, `SHA-256`, `USB CDC`)
- [ ] Zero-install WebSerial live visualizers (`stamps` & `architecture graph`)
- [ ] Pre-built `.uf2` release & entropy benchmarks

## License

[MIT](LICENSE)
