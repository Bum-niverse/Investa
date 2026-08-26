use serde::{Deserialize, Serialize};

use crate::trading::TradeSide;

const THOUSANDTH_BASIS_POINTS: u128 = 10_000_000;
const RATE_PRECISION: f64 = 1_000.0;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradingCosts {
    pub buy_fee_bps: f64,
    pub sell_fee_bps: f64,
    pub sell_tax_bps: f64,
    pub slippage_bps: f64,
}

pub const KR_STOCK_DEFAULT_COSTS: TradingCosts = TradingCosts {
    buy_fee_bps: 1.5,
    sell_fee_bps: 1.5,
    sell_tax_bps: 20.0,
    slippage_bps: 0.0,
};

pub const US_STOCK_DEFAULT_COSTS: TradingCosts = TradingCosts {
    buy_fee_bps: 10.0,
    sell_fee_bps: 10.0,
    sell_tax_bps: 0.206,
    slippage_bps: 0.0,
};

pub const UPBIT_KRW_DEFAULT_COSTS: TradingCosts = TradingCosts {
    buy_fee_bps: 5.0,
    sell_fee_bps: 5.0,
    sell_tax_bps: 0.0,
    slippage_bps: 0.0,
};

pub fn default_stock_costs(currency: &str) -> Result<TradingCosts, CostError> {
    match currency {
        "KRW" => Ok(KR_STOCK_DEFAULT_COSTS),
        "USD" => Ok(US_STOCK_DEFAULT_COSTS),
        _ => Err(CostError::InvalidCosts),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionQuote {
    pub execution_price_minor: u64,
    pub notional_minor: u64,
    pub fee_minor: u64,
    pub tax_minor: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostError {
    InvalidCosts,
    InvalidOrder,
    Overflow,
}

pub fn validate_costs(costs: TradingCosts) -> Result<(), CostError> {
    for (rate, allow_full_notional) in [
        (costs.buy_fee_bps, true),
        (costs.sell_fee_bps, true),
        (costs.sell_tax_bps, true),
        (costs.slippage_bps, false),
    ] {
        let scaled = rate * RATE_PRECISION;
        if !rate.is_finite()
            || rate < 0.0
            || if allow_full_notional {
                rate > 10_000.0
            } else {
                rate >= 10_000.0
            }
            || (scaled - scaled.round()).abs() > 1e-7
        {
            return Err(CostError::InvalidCosts);
        }
    }
    Ok(())
}

fn ceil_div(value: u128, divisor: u128) -> Result<u128, CostError> {
    value
        .checked_add(divisor - 1)
        .map(|adjusted| adjusted / divisor)
        .ok_or(CostError::Overflow)
}

fn thousandth_bps(rate: f64) -> Result<u128, CostError> {
    let scaled = rate * RATE_PRECISION;
    if !scaled.is_finite() || scaled < 0.0 || scaled > u128::MAX as f64 {
        return Err(CostError::InvalidCosts);
    }
    Ok(scaled.round() as u128)
}

fn charge(notional_minor: u64, bps: f64) -> Result<u64, CostError> {
    let value = ceil_div(
        u128::from(notional_minor) * thousandth_bps(bps)?,
        THOUSANDTH_BASIS_POINTS,
    )?;
    u64::try_from(value).map_err(|_| CostError::Overflow)
}

pub fn quote_execution(
    side: TradeSide,
    reference_price_minor: u64,
    quantity: u64,
    costs: TradingCosts,
) -> Result<ExecutionQuote, CostError> {
    quote_execution_scaled(side, reference_price_minor, quantity, 1, costs)
}

pub fn quote_execution_scaled(
    side: TradeSide,
    reference_price_minor: u64,
    quantity: u64,
    quantity_scale: u64,
    costs: TradingCosts,
) -> Result<ExecutionQuote, CostError> {
    validate_costs(costs)?;
    if reference_price_minor == 0 || quantity == 0 || quantity_scale == 0 {
        return Err(CostError::InvalidOrder);
    }

    let reference = u128::from(reference_price_minor);
    let slippage = thousandth_bps(costs.slippage_bps)?;
    let execution_price = match side {
        TradeSide::Buy => ceil_div(
            reference * (THOUSANDTH_BASIS_POINTS + slippage),
            THOUSANDTH_BASIS_POINTS,
        )?,
        TradeSide::Sell => {
            reference
                .checked_mul(THOUSANDTH_BASIS_POINTS - slippage)
                .ok_or(CostError::Overflow)?
                / THOUSANDTH_BASIS_POINTS
        }
    };
    let execution_price_minor = u64::try_from(execution_price).map_err(|_| CostError::Overflow)?;
    if execution_price_minor == 0 {
        return Err(CostError::InvalidOrder);
    }

    let scaled_notional = u128::from(execution_price_minor)
        .checked_mul(u128::from(quantity))
        .ok_or(CostError::Overflow)?;
    let notional_minor = u64::try_from(match side {
        TradeSide::Buy => ceil_div(scaled_notional, u128::from(quantity_scale))?,
        TradeSide::Sell => scaled_notional / u128::from(quantity_scale),
    })
    .map_err(|_| CostError::Overflow)?;
    if notional_minor == 0 {
        return Err(CostError::InvalidOrder);
    }
    let fee_bps = match side {
        TradeSide::Buy => costs.buy_fee_bps,
        TradeSide::Sell => costs.sell_fee_bps,
    };

    Ok(ExecutionQuote {
        execution_price_minor,
        notional_minor,
        fee_minor: charge(notional_minor, fee_bps)?,
        tax_minor: match side {
            TradeSide::Buy => 0,
            TradeSide::Sell => charge(notional_minor, costs.sell_tax_bps)?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_adverse_slippage_and_rounds_charges_up() {
        let costs = TradingCosts {
            buy_fee_bps: 1.0,
            sell_fee_bps: 1.0,
            sell_tax_bps: 15.0,
            slippage_bps: 10.0,
        };

        let buy = quote_execution(TradeSide::Buy, 10_001, 3, costs).unwrap();
        let sell = quote_execution(TradeSide::Sell, 10_001, 3, costs).unwrap();

        assert_eq!(buy.execution_price_minor, 10_012);
        assert_eq!(sell.execution_price_minor, 9_990);
        assert_eq!(buy.fee_minor, 4);
        assert_eq!(sell.fee_minor, 3);
        assert_eq!(sell.tax_minor, 45);
    }

    #[test]
    fn rejects_zero_orders_and_impossible_cost_inputs() {
        let invalid = TradingCosts {
            buy_fee_bps: 0.0,
            sell_fee_bps: 0.0,
            sell_tax_bps: 0.0,
            slippage_bps: 10_000.0,
        };

        assert_eq!(
            quote_execution(TradeSide::Buy, 10_000, 1, invalid),
            Err(CostError::InvalidCosts)
        );
        assert_eq!(
            quote_execution(
                TradeSide::Buy,
                0,
                1,
                TradingCosts {
                    buy_fee_bps: 0.0,
                    sell_fee_bps: 0.0,
                    sell_tax_bps: 0.0,
                    slippage_bps: 0.0,
                },
            ),
            Err(CostError::InvalidOrder)
        );
    }

    #[test]
    fn preserves_official_fractional_basis_point_rates() {
        let costs = TradingCosts {
            buy_fee_bps: 1.5,
            sell_fee_bps: 1.5,
            sell_tax_bps: 0.206,
            slippage_bps: 0.0,
        };

        let quote = quote_execution(TradeSide::Sell, 10_000_000, 1, costs).unwrap();
        assert_eq!(quote.fee_minor, 1_500);
        assert_eq!(quote.tax_minor, 206);

        assert_eq!(
            validate_costs(TradingCosts {
                buy_fee_bps: 1.2345,
                ..costs
            }),
            Err(CostError::InvalidCosts)
        );
    }

    #[test]
    fn reads_existing_integer_bps_json_as_fractional_costs() {
        let costs: TradingCosts = serde_json::from_str(
            r#"{"buyFeeBps":5,"sellFeeBps":5,"sellTaxBps":20,"slippageBps":0}"#,
        )
        .unwrap();
        assert_eq!(costs.buy_fee_bps, 5.0);
        assert_eq!(costs.sell_tax_bps, 20.0);
    }

    #[test]
    fn selects_official_stock_defaults_by_currency() {
        assert_eq!(default_stock_costs("KRW").unwrap(), KR_STOCK_DEFAULT_COSTS);
        assert_eq!(default_stock_costs("USD").unwrap(), US_STOCK_DEFAULT_COSTS);
        assert_eq!(default_stock_costs("BTC"), Err(CostError::InvalidCosts));
    }
}
