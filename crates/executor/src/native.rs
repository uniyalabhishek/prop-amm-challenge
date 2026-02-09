use prop_amm_shared::instruction::encode_instruction;

/// A swap function signature: takes 25-byte instruction data, returns output amount.
pub type SwapFn = fn(&[u8]) -> u64;

/// Native executor that calls a Rust function directly (no BPF overhead).
#[derive(Clone)]
pub struct NativeExecutor {
    swap_fn: SwapFn,
}

impl NativeExecutor {
    pub fn new(swap_fn: SwapFn) -> Self {
        Self { swap_fn }
    }

    #[inline]
    pub fn execute(&self, side: u8, amount: u64, rx: u64, ry: u64) -> u64 {
        let data = encode_instruction(side, amount, rx, ry);
        (self.swap_fn)(&data)
    }
}
