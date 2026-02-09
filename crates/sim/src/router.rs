use crate::amm::BpfAmm;
use crate::retail::RetailOrder;

pub struct RoutedTrade {
    pub is_submission: bool,
    pub amm_buys_x: bool,
    pub amount_x: f64,
    pub amount_y: f64,
}

const GRID_POINTS: usize = 11;

pub struct OrderRouter;

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
        let mut best_alpha = 0.0_f64;
        let mut best_output = 0.0_f64;

        for i in 0..GRID_POINTS {
            let alpha = i as f64 / (GRID_POINTS - 1) as f64;
            let y_sub = total_y * alpha;
            let y_norm = total_y * (1.0 - alpha);

            let x_sub = if y_sub > 0.001 { amm_sub.quote_buy_x(y_sub) } else { 0.0 };
            let x_norm = if y_norm > 0.001 { amm_norm.quote_buy_x(y_norm) } else { 0.0 };
            let total_x = x_sub + x_norm;

            if total_x > best_output {
                best_output = total_x;
                best_alpha = alpha;
            }
        }

        let mut trades = Vec::new();
        let y_sub = total_y * best_alpha;
        let y_norm = total_y * (1.0 - best_alpha);

        if y_sub > 0.001 {
            let x_out = amm_sub.execute_buy_x(y_sub);
            if x_out > 0.0 {
                trades.push(RoutedTrade { is_submission: true, amm_buys_x: false, amount_x: x_out, amount_y: y_sub });
            }
        }
        if y_norm > 0.001 {
            let x_out = amm_norm.execute_buy_x(y_norm);
            if x_out > 0.0 {
                trades.push(RoutedTrade { is_submission: false, amm_buys_x: false, amount_x: x_out, amount_y: y_norm });
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
        let mut best_alpha = 0.0_f64;
        let mut best_output = 0.0_f64;

        for i in 0..GRID_POINTS {
            let alpha = i as f64 / (GRID_POINTS - 1) as f64;
            let x_sub = total_x * alpha;
            let x_norm = total_x * (1.0 - alpha);

            let y_sub = if x_sub > 0.001 { amm_sub.quote_sell_x(x_sub) } else { 0.0 };
            let y_norm = if x_norm > 0.001 { amm_norm.quote_sell_x(x_norm) } else { 0.0 };
            let total_y = y_sub + y_norm;

            if total_y > best_output {
                best_output = total_y;
                best_alpha = alpha;
            }
        }

        let mut trades = Vec::new();
        let x_sub = total_x * best_alpha;
        let x_norm = total_x * (1.0 - best_alpha);

        if x_sub > 0.001 {
            let y_out = amm_sub.execute_sell_x(x_sub);
            if y_out > 0.0 {
                trades.push(RoutedTrade { is_submission: true, amm_buys_x: true, amount_x: x_sub, amount_y: y_out });
            }
        }
        if x_norm > 0.001 {
            let y_out = amm_norm.execute_sell_x(x_norm);
            if y_out > 0.0 {
                trades.push(RoutedTrade { is_submission: false, amm_buys_x: true, amount_x: x_norm, amount_y: y_out });
            }
        }
        trades
    }
}
