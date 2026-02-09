use prop_amm_executor::{BpfExecutor, BpfProgram, NativeExecutor, SwapFn};
use prop_amm_shared::nano::{f64_to_nano, nano_to_f64};

enum Backend {
    Bpf(BpfExecutor),
    Native(NativeExecutor),
}

pub struct BpfAmm {
    backend: Backend,
    pub reserve_x: f64,
    pub reserve_y: f64,
    pub name: String,
}

impl BpfAmm {
    pub fn new(program: BpfProgram, reserve_x: f64, reserve_y: f64, name: String) -> Self {
        Self {
            backend: Backend::Bpf(BpfExecutor::new(program)),
            reserve_x,
            reserve_y,
            name,
        }
    }

    pub fn new_native(swap_fn: SwapFn, reserve_x: f64, reserve_y: f64, name: String) -> Self {
        Self {
            backend: Backend::Native(NativeExecutor::new(swap_fn)),
            reserve_x,
            reserve_y,
            name,
        }
    }

    #[inline]
    fn call(&mut self, side: u8, amount: u64, rx: u64, ry: u64) -> u64 {
        match &mut self.backend {
            Backend::Bpf(exec) => exec.execute(side, amount, rx, ry).unwrap_or(0),
            Backend::Native(exec) => exec.execute(side, amount, rx, ry),
        }
    }

    #[inline]
    pub fn quote_buy_x(&mut self, input_y: f64) -> f64 {
        if input_y <= 0.0 { return 0.0; }
        nano_to_f64(self.call(0, f64_to_nano(input_y), f64_to_nano(self.reserve_x), f64_to_nano(self.reserve_y)))
    }

    #[inline]
    pub fn quote_sell_x(&mut self, input_x: f64) -> f64 {
        if input_x <= 0.0 { return 0.0; }
        nano_to_f64(self.call(1, f64_to_nano(input_x), f64_to_nano(self.reserve_x), f64_to_nano(self.reserve_y)))
    }

    #[inline]
    pub fn execute_buy_x(&mut self, input_y: f64) -> f64 {
        let output_x = self.quote_buy_x(input_y);
        if output_x > 0.0 {
            self.reserve_x -= output_x;
            self.reserve_y += input_y;
        }
        output_x
    }

    #[inline]
    pub fn execute_sell_x(&mut self, input_x: f64) -> f64 {
        let output_y = self.quote_sell_x(input_x);
        if output_y > 0.0 {
            self.reserve_x += input_x;
            self.reserve_y -= output_y;
        }
        output_y
    }

    #[inline]
    pub fn spot_price(&self) -> f64 {
        self.reserve_y / self.reserve_x
    }

    pub fn reset(&mut self, reserve_x: f64, reserve_y: f64) {
        self.reserve_x = reserve_x;
        self.reserve_y = reserve_y;
    }
}
