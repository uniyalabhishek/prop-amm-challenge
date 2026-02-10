use crate::amm::BpfAmm;
use crate::retail::RetailOrder;

pub struct RoutedTrade {
    pub is_submission: bool,
    pub amm_buys_x: bool,
    pub amount_x: f64,
    pub amount_y: f64,
}

const GRID_POINTS: usize = 11;
const MIN_TRADE_SIZE: f64 = 0.001;
const CURVE_EPS: f64 = 1e-9;

pub struct OrderRouter;

#[derive(Clone, Copy)]
struct QuotePoint {
    alpha: f64,
    in_sub: f64,
    in_norm: f64,
    out_sub: f64,
    out_norm: f64,
}

impl OrderRouter {
    pub fn new() -> Self {
        Self
    }

    pub fn route_order(
        &self,
        order: &RetailOrder,
        amm_sub: &mut BpfAmm,
        amm_norm: &mut BpfAmm,
        fair_price: f64,
    ) -> Vec<RoutedTrade> {
        if order.is_buy {
            self.route_buy(order.size, amm_sub, amm_norm)
        } else {
            let total_x = order.size / fair_price;
            self.route_sell(total_x, amm_sub, amm_norm)
        }
    }

    fn route_buy(
        &self,
        total_y: f64,
        amm_sub: &mut BpfAmm,
        amm_norm: &mut BpfAmm,
    ) -> Vec<RoutedTrade> {
        let mut points = Vec::with_capacity(GRID_POINTS);

        for i in 0..GRID_POINTS {
            let alpha = i as f64 / (GRID_POINTS - 1) as f64;
            let y_sub = total_y * alpha;
            let y_norm = total_y * (1.0 - alpha);

            let x_sub = if y_sub > MIN_TRADE_SIZE {
                amm_sub.quote_buy_x(y_sub)
            } else {
                0.0
            };
            let x_norm = if y_norm > MIN_TRADE_SIZE {
                amm_norm.quote_buy_x(y_norm)
            } else {
                0.0
            };

            points.push(QuotePoint {
                alpha,
                in_sub: y_sub,
                in_norm: y_norm,
                out_sub: x_sub,
                out_norm: x_norm,
            });
        }

        let sub_curve: Vec<(f64, f64)> = points.iter().map(|p| (p.in_sub, p.out_sub)).collect();
        let norm_curve: Vec<(f64, f64)> = points.iter().map(|p| (p.in_norm, p.out_norm)).collect();
        let sub_valid = Self::is_valid_curve(&sub_curve, amm_sub.reserve_x);
        let norm_valid = Self::is_valid_curve(&norm_curve, amm_norm.reserve_x);

        let best = points
            .iter()
            .filter(|p| Self::split_is_allowed(p, sub_valid, norm_valid))
            .max_by(|a, b| {
                let a_total = a.out_sub + a.out_norm;
                let b_total = b.out_sub + b.out_norm;
                a_total.total_cmp(&b_total)
            })
            .copied();

        let Some(best) = best else {
            return Vec::new();
        };

        if (best.alpha - 0.0).abs() < CURVE_EPS && !norm_valid {
            return Vec::new();
        }
        if (best.alpha - 1.0).abs() < CURVE_EPS && !sub_valid {
            return Vec::new();
        }

        let mut trades = Vec::new();
        let y_sub = best.in_sub;
        let y_norm = best.in_norm;

        if y_sub > MIN_TRADE_SIZE && sub_valid {
            let x_out = amm_sub.execute_buy_x_quoted(y_sub, best.out_sub);
            if x_out > 0.0 {
                trades.push(RoutedTrade {
                    is_submission: true,
                    amm_buys_x: false,
                    amount_x: x_out,
                    amount_y: y_sub,
                });
            }
        }
        if y_norm > MIN_TRADE_SIZE && norm_valid {
            let x_out = amm_norm.execute_buy_x_quoted(y_norm, best.out_norm);
            if x_out > 0.0 {
                trades.push(RoutedTrade {
                    is_submission: false,
                    amm_buys_x: false,
                    amount_x: x_out,
                    amount_y: y_norm,
                });
            }
        }
        trades
    }

    fn route_sell(
        &self,
        total_x: f64,
        amm_sub: &mut BpfAmm,
        amm_norm: &mut BpfAmm,
    ) -> Vec<RoutedTrade> {
        let mut points = Vec::with_capacity(GRID_POINTS);

        for i in 0..GRID_POINTS {
            let alpha = i as f64 / (GRID_POINTS - 1) as f64;
            let x_sub = total_x * alpha;
            let x_norm = total_x * (1.0 - alpha);

            let y_sub = if x_sub > MIN_TRADE_SIZE {
                amm_sub.quote_sell_x(x_sub)
            } else {
                0.0
            };
            let y_norm = if x_norm > MIN_TRADE_SIZE {
                amm_norm.quote_sell_x(x_norm)
            } else {
                0.0
            };

            points.push(QuotePoint {
                alpha,
                in_sub: x_sub,
                in_norm: x_norm,
                out_sub: y_sub,
                out_norm: y_norm,
            });
        }

        let sub_curve: Vec<(f64, f64)> = points.iter().map(|p| (p.in_sub, p.out_sub)).collect();
        let norm_curve: Vec<(f64, f64)> = points.iter().map(|p| (p.in_norm, p.out_norm)).collect();
        let sub_valid = Self::is_valid_curve(&sub_curve, amm_sub.reserve_y);
        let norm_valid = Self::is_valid_curve(&norm_curve, amm_norm.reserve_y);

        let best = points
            .iter()
            .filter(|p| Self::split_is_allowed(p, sub_valid, norm_valid))
            .max_by(|a, b| {
                let a_total = a.out_sub + a.out_norm;
                let b_total = b.out_sub + b.out_norm;
                a_total.total_cmp(&b_total)
            })
            .copied();

        let Some(best) = best else {
            return Vec::new();
        };

        if (best.alpha - 0.0).abs() < CURVE_EPS && !norm_valid {
            return Vec::new();
        }
        if (best.alpha - 1.0).abs() < CURVE_EPS && !sub_valid {
            return Vec::new();
        }

        let mut trades = Vec::new();
        let x_sub = best.in_sub;
        let x_norm = best.in_norm;

        if x_sub > MIN_TRADE_SIZE && sub_valid {
            let y_out = amm_sub.execute_sell_x_quoted(x_sub, best.out_sub);
            if y_out > 0.0 {
                trades.push(RoutedTrade {
                    is_submission: true,
                    amm_buys_x: true,
                    amount_x: x_sub,
                    amount_y: y_out,
                });
            }
        }
        if x_norm > MIN_TRADE_SIZE && norm_valid {
            let y_out = amm_norm.execute_sell_x_quoted(x_norm, best.out_norm);
            if y_out > 0.0 {
                trades.push(RoutedTrade {
                    is_submission: false,
                    amm_buys_x: true,
                    amount_x: x_norm,
                    amount_y: y_out,
                });
            }
        }
        trades
    }

    fn split_is_allowed(point: &QuotePoint, sub_valid: bool, norm_valid: bool) -> bool {
        if !sub_valid && point.in_sub > MIN_TRADE_SIZE {
            return false;
        }
        if !norm_valid && point.in_norm > MIN_TRADE_SIZE {
            return false;
        }
        true
    }

    fn is_valid_curve(points: &[(f64, f64)], max_output: f64) -> bool {
        if !max_output.is_finite() || max_output <= 0.0 {
            return false;
        }

        let mut prev_input = None;
        let mut prev_output = None;
        let mut prev_slope = f64::INFINITY;

        for (input, output) in points {
            if *input < 0.0 || !input.is_finite() || !output.is_finite() {
                return false;
            }
            if *output < -CURVE_EPS || *output > max_output + CURVE_EPS {
                return false;
            }

            if let Some(last_output) = prev_output {
                if *output + CURVE_EPS < last_output {
                    return false;
                }
            }

            if let (Some(last_input), Some(last_output)) = (prev_input, prev_output) {
                let delta_input = *input - last_input;
                if delta_input > CURVE_EPS {
                    let slope = (*output - last_output) / delta_input;
                    if slope < -CURVE_EPS {
                        return false;
                    }
                    if slope > prev_slope + CURVE_EPS {
                        return false;
                    }
                    prev_slope = slope;
                }
            }

            prev_input = Some(*input);
            prev_output = Some(*output);
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::OrderRouter;
    use crate::amm::BpfAmm;
    use crate::retail::RetailOrder;
    use prop_amm_shared::normalizer::compute_swap as normalizer_swap;
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static CALL_COUNTS: OnceLock<Mutex<HashMap<(u8, u64, u64, u64), u32>>> = OnceLock::new();

    fn quote_bait_swap(data: &[u8]) -> u64 {
        if data.len() < 25 {
            return 0;
        }
        let side = data[0];
        let input = u64::from_le_bytes(data[1..9].try_into().unwrap());
        let rx = u64::from_le_bytes(data[9..17].try_into().unwrap());
        let ry = u64::from_le_bytes(data[17..25].try_into().unwrap());
        let key = (side, input, rx, ry);

        let counts = CALL_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut counts = counts.lock().unwrap();
        let entry = counts.entry(key).or_insert(0);
        *entry += 1;

        if *entry == 1 {
            normalizer_swap(data).saturating_add(100)
        } else {
            1
        }
    }

    #[test]
    fn router_executes_committed_quote_output() {
        let counts = CALL_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
        counts.lock().unwrap().clear();

        let router = OrderRouter::new();
        let mut amm_sub =
            BpfAmm::new_native(quote_bait_swap, None, 100.0, 10_000.0, "sub".to_string());
        let mut amm_norm =
            BpfAmm::new_native(normalizer_swap, None, 100.0, 10_000.0, "norm".to_string());
        let order = RetailOrder {
            is_buy: true,
            size: 20.0,
        };

        let trades = router.route_order(&order, &mut amm_sub, &mut amm_norm, 100.0);
        let sub_trade = trades
            .iter()
            .find(|t| t.is_submission)
            .expect("submission leg should execute");

        // With quote-commit, execution must use the selected quote output and not re-quote.
        assert!(
            sub_trade.amount_x > 0.05,
            "unexpectedly tiny execution output: {}",
            sub_trade.amount_x
        );
    }
}
