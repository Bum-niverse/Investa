use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::State;

use crate::{
    paper_account::{open_paper_account, replay_ledger, AppendOnlyLedger, PaperAccountState},
    persistence::PersistenceBridge,
};

pub const PAPER_LEDGER_ID: &str = "paper-krw-v1";
pub const PAPER_ACCOUNT_ID: &str = "investa-paper-krw";
pub const DEFAULT_INITIAL_CASH_KRW: u64 = 100_000_000;
pub const PAPER_LEDGER_ID_USD: &str = "paper-usd-v1";
pub const PAPER_ACCOUNT_ID_USD: &str = "investa-paper-usd";
pub const DEFAULT_INITIAL_CASH_USD_MINOR: u64 = 10_000_000;
pub const LIVE_ORDER_ENABLED: bool = false;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperAccountSnapshot {
    pub mode: &'static str,
    pub live_order_enabled: bool,
    pub initial_cash_minor: u64,
    pub account: PaperAccountState,
    pub warning: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderAdapterStatus {
    pub mode: &'static str,
    pub live_order_enabled: bool,
    pub order_transport_compiled: bool,
    pub message: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperAccountsSnapshot {
    pub accounts: Vec<PaperAccountSnapshot>,
    pub live_order_enabled: bool,
}

pub(crate) fn now_ms() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "현재 시각을 확인하지 못했습니다.".to_owned())?
        .as_millis()
        .try_into()
        .map_err(|_| "현재 시각이 지원 범위를 초과했습니다.".to_owned())
}

pub(crate) fn load_or_open_account(
    persistence: &PersistenceBridge,
) -> Result<PaperAccountState, String> {
    let mut ledger = persistence.paper_ledger(PAPER_LEDGER_ID)?;
    if ledger.events().is_empty() {
        open_paper_account(
            &mut ledger,
            PAPER_ACCOUNT_ID.to_owned(),
            "KRW".to_owned(),
            DEFAULT_INITIAL_CASH_KRW,
            now_ms()?,
        )
        .map_err(|error| error.message)
    } else {
        replay_ledger(ledger.events()).map_err(|error| error.message)
    }
}

pub(crate) fn load_or_open_account_for_currency(
    persistence: &PersistenceBridge,
    currency: &str,
) -> Result<PaperAccountState, String> {
    let (ledger_id, account_id, initial_cash_minor) = match currency {
        "KRW" => (PAPER_LEDGER_ID, PAPER_ACCOUNT_ID, DEFAULT_INITIAL_CASH_KRW),
        "USD" => (
            PAPER_LEDGER_ID_USD,
            PAPER_ACCOUNT_ID_USD,
            DEFAULT_INITIAL_CASH_USD_MINOR,
        ),
        _ => return Err("현재 내부 모의계좌는 KRW와 USD만 지원합니다.".to_owned()),
    };
    let mut ledger = persistence.paper_ledger(ledger_id)?;
    if ledger.events().is_empty() {
        open_paper_account(
            &mut ledger,
            account_id.to_owned(),
            currency.to_owned(),
            initial_cash_minor,
            now_ms()?,
        )
        .map_err(|error| error.message)
    } else {
        replay_ledger(ledger.events()).map_err(|error| error.message)
    }
}

pub(crate) fn ledger_id_for_currency(currency: &str) -> Result<&'static str, String> {
    match currency {
        "KRW" => Ok(PAPER_LEDGER_ID),
        "USD" => Ok(PAPER_LEDGER_ID_USD),
        _ => Err("현재 내부 모의계좌는 KRW와 USD만 지원합니다.".to_owned()),
    }
}

pub(crate) fn snapshot(account: PaperAccountState) -> PaperAccountSnapshot {
    let initial_cash_minor = match account.currency.as_str() {
        "USD" => DEFAULT_INITIAL_CASH_USD_MINOR,
        _ => DEFAULT_INITIAL_CASH_KRW,
    };
    PaperAccountSnapshot {
        mode: "shadow_only",
        live_order_enabled: LIVE_ORDER_ENABLED,
        initial_cash_minor,
        account,
        warning: "내부 모의체결 결과이며 토스증권 계좌나 실제 주문에 반영되지 않습니다.",
    }
}

#[tauri::command]
pub fn paper_account_status(
    persistence: State<'_, PersistenceBridge>,
) -> Result<PaperAccountSnapshot, String> {
    load_or_open_account(&persistence).map(snapshot)
}

#[tauri::command]
pub fn paper_accounts_status(
    persistence: State<'_, PersistenceBridge>,
) -> Result<PaperAccountsSnapshot, String> {
    let krw = load_or_open_account_for_currency(&persistence, "KRW")?;
    let usd = load_or_open_account_for_currency(&persistence, "USD")?;
    Ok(PaperAccountsSnapshot {
        accounts: vec![snapshot(krw), snapshot(usd)],
        live_order_enabled: LIVE_ORDER_ENABLED,
    })
}

#[tauri::command]
pub fn toss_order_adapter_status() -> OrderAdapterStatus {
    OrderAdapterStatus {
        mode: "read_only_plus_shadow",
        live_order_enabled: LIVE_ORDER_ENABLED,
        order_transport_compiled: false,
        message:
            "토스 실주문 전송 코드는 포함하지 않았습니다. 주문 후보는 내부 모의원장으로만 보냅니다.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_the_approved_krw_paper_account_idempotently() {
        let persistence = PersistenceBridge::in_memory().expect("database");
        let first = load_or_open_account(&persistence).expect("first open");
        let second = load_or_open_account(&persistence).expect("reopen");
        assert_eq!(first.cash_minor, 100_000_000);
        assert_eq!(second.cash_minor, first.cash_minor);
        assert_eq!(second.event_count, 1);
    }

    #[test]
    fn opens_separate_krw_and_usd_ledgers() {
        let persistence = PersistenceBridge::in_memory().expect("database");
        let krw = load_or_open_account_for_currency(&persistence, "KRW").expect("krw");
        let usd = load_or_open_account_for_currency(&persistence, "USD").expect("usd");
        assert_eq!(krw.currency, "KRW");
        assert_eq!(krw.cash_minor, DEFAULT_INITIAL_CASH_KRW);
        assert_eq!(usd.currency, "USD");
        assert_eq!(usd.cash_minor, DEFAULT_INITIAL_CASH_USD_MINOR);
        assert_ne!(krw.account_id, usd.account_id);
    }

    #[test]
    fn live_order_transport_is_a_compile_time_closed_boundary() {
        let status = toss_order_adapter_status();
        assert!(!status.live_order_enabled);
        assert!(!status.order_transport_compiled);
    }
}
