use crate::amm::BpfAmm;

pub struct ArbResult {
    pub amm_buys_x: bool,
    pub amount_x: f64,
    pub amount_y: f64,
    pub edge: f64,
}

pub struct Arbitrageur;

impl Arbitrageur {
    pub fn new() -> Self {
        Self
    }

    pub fn execute_arb(&self, amm: &mut BpfAmm, fair_price: f64) -> Option<ArbResult> {
        let spot = amm.spot_price();

        if spot < fair_price * 0.9999 {
            self.arb_buy_x(amm, fair_price)
        } else if spot > fair_price * 1.0001 {
            self.arb_sell_x(amm, fair_price)
        } else {
            None
        }
    }

    fn arb_buy_x(&self, amm: &mut BpfAmm, fair_price: f64) -> Option<ArbResult> {
        let mut lo = 0.0_f64;
        let mut hi = amm.reserve_y * 0.5;

        for _ in 0..12 {
            let mid = (lo + hi) / 2.0;
            let eps = mid * 0.001 + 0.001;
            let out_lo = amm.quote_buy_x(mid);
            let out_hi = amm.quote_buy_x(mid + eps);
            let marginal_x = (out_hi - out_lo) / eps;
            if marginal_x * fair_price > 1.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }

        let optimal_y = (lo + hi) / 2.0;
        if optimal_y < 0.001 {
            return None;
        }

        let output_x = amm.execute_buy_x(optimal_y);
        if output_x <= 0.0 {
            return None;
        }

        Some(ArbResult {
            amm_buys_x: false,
            amount_x: output_x,
            amount_y: optimal_y,
            edge: optimal_y - output_x * fair_price,
        })
    }

    fn arb_sell_x(&self, amm: &mut BpfAmm, fair_price: f64) -> Option<ArbResult> {
        let mut lo = 0.0_f64;
        let mut hi = amm.reserve_x * 0.5;

        for _ in 0..12 {
            let mid = (lo + hi) / 2.0;
            let eps = mid * 0.001 + 0.001;
            let out_lo = amm.quote_sell_x(mid);
            let out_hi = amm.quote_sell_x(mid + eps);
            let marginal_y = (out_hi - out_lo) / eps;
            if marginal_y > fair_price {
                lo = mid;
            } else {
                hi = mid;
            }
        }

        let optimal_x = (lo + hi) / 2.0;
        if optimal_x < 0.001 {
            return None;
        }

        let output_y = amm.execute_sell_x(optimal_x);
        if output_y <= 0.0 {
            return None;
        }

        Some(ArbResult {
            amm_buys_x: true,
            amount_x: optimal_x,
            amount_y: output_y,
            edge: optimal_x * fair_price - output_y,
        })
    }
}
