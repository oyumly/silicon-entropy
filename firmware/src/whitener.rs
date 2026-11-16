use sha2::{Digest, Sha256};

pub struct Whitener {
    counter: u64,
}

impl Whitener {
    pub const fn new() -> Self {
        Self { counter: 0 }
    }

    pub fn whiten_block(&mut self, raw_64: &[u8; 64]) -> [u8; 32] {
        self.counter = self.counter.wrapping_add(1);
        let mut hasher = Sha256::new();
        hasher.update(raw_64);
        hasher.update(self.counter.to_le_bytes());
        let result = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&result);
        out
    }
}
