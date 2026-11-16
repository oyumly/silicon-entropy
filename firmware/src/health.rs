// NIST SP 800-90B continuous health test thresholds
const APT_WINDOW_BITS: u32 = 1024;
const APT_MIN_ONES: u32 = 350;
const APT_MAX_ONES: u32 = 674;
const RCT_BIT_LIMIT: u32 = 48;

#[derive(Clone, Copy)]
pub struct HealthStats {
    pub healthy: bool,
    pub total_bytes: u32,
    pub rct_failures: u32,
    pub apt_failures: u32,
    pub last_ones_per_1k: u32,
}

pub struct HealthMonitor {
    last_bit: u8,
    run_len: u32,
    apt_bits: u32,
    apt_ones: u32,
    pub stats: HealthStats,
}

impl HealthMonitor {
    pub const fn new() -> Self {
        Self {
            last_bit: 0,
            run_len: 0,
            apt_bits: 0,
            apt_ones: 0,
            stats: HealthStats {
                healthy: true,
                total_bytes: 0,
                rct_failures: 0,
                apt_failures: 0,
                last_ones_per_1k: 512,
            },
        }
    }

    pub fn feed_byte(&mut self, byte: u8) -> bool {
        self.stats.total_bytes = self.stats.total_bytes.wrapping_add(1);
        let mut ok = true;

        for i in 0..8 {
            let bit = (byte >> i) & 1;

            if bit == self.last_bit {
                self.run_len += 1;
                if self.run_len >= RCT_BIT_LIMIT {
                    self.stats.rct_failures = self.stats.rct_failures.wrapping_add(1);
                    ok = false;
                }
            } else {
                self.last_bit = bit;
                self.run_len = 1;
            }

            self.apt_bits += 1;
            self.apt_ones += bit as u32;
            if self.apt_bits >= APT_WINDOW_BITS {
                self.stats.last_ones_per_1k = self.apt_ones;
                if self.apt_ones < APT_MIN_ONES || self.apt_ones > APT_MAX_ONES {
                    self.stats.apt_failures = self.stats.apt_failures.wrapping_add(1);
                    ok = false;
                }
                self.apt_bits = 0;
                self.apt_ones = 0;
            }
        }

        self.stats.healthy = ok;
        ok
    }
}
