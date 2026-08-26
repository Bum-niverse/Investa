use keyring::{Entry, Error as KeyringError};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::State;

const TICKERS_URL: &str = "https://www.sec.gov/files/company_tickers.json";
const COMPANY_FACTS_BASE_URL: &str = "https://data.sec.gov/api/xbrl/companyfacts";
const SUBMISSIONS_BASE_URL: &str = "https://data.sec.gov/submissions";
const CREDENTIAL_SERVICE: &str = "com.bumniverse.investa.sec-data";
const CONTACT_ACCOUNT: &str = "contact-email";
const REQUEST_TIMEOUT_SECONDS: u64 = 10;
const TICKER_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecConnectionStatus {
    configured: bool,
    connected: bool,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecContactRequest {
    contact_email: String,
}

#[derive(Clone, Debug, Deserialize)]
struct SecTickerEntry {
    cik_str: u64,
    ticker: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecFundamentalMetric {
    key: String,
    label: String,
    value: String,
    unit: String,
    period_start: Option<String>,
    period_end: String,
    filed_at: String,
    form: String,
    accession_no: String,
    fiscal_year: Option<i64>,
    fiscal_period: Option<String>,
    frame: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecFundamentalSnapshot {
    provider: &'static str,
    cik: String,
    ticker: String,
    entity_name: String,
    as_of_date: String,
    pub(crate) metrics: Vec<SecFundamentalMetric>,
    pub(crate) missing_metrics: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecFilingRecord {
    accession_no: String,
    form: String,
    filed_at: String,
    report_date: Option<String>,
    primary_document: Option<String>,
    description: Option<String>,
    items: Vec<String>,
    filing_index_url: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecFilingSnapshot {
    provider: &'static str,
    cik: String,
    ticker: String,
    entity_name: String,
    as_of_date: String,
    pub(crate) filings: Vec<SecFilingRecord>,
}

struct TickerCache {
    fetched_at: Instant,
    entries: HashMap<String, SecTickerEntry>,
}

pub struct SecFundamentalsBridge {
    client: Client,
    ticker_cache: Mutex<Option<TickerCache>>,
}

impl Default for SecFundamentalsBridge {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
                .build()
                .expect("SEC HTTP client"),
            ticker_cache: Mutex::new(None),
        }
    }
}

fn contact_entry() -> Result<Entry, String> {
    Entry::new(CREDENTIAL_SERVICE, CONTACT_ACCOUNT)
        .map_err(|_| "SEC 연락처 보안 저장소를 열지 못했습니다.".to_owned())
}

fn load_contact() -> Result<Option<String>, String> {
    match contact_entry()?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err("SEC 연락처를 Windows 자격 증명 관리자에서 읽지 못했습니다.".to_owned()),
    }
}

fn validate_contact(value: &str) -> Result<String, String> {
    let contact = value.trim();
    let valid = contact.len() <= 254
        && !contact.is_empty()
        && !contact.chars().any(char::is_control)
        && contact.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.contains('.') && !domain.starts_with('.')
        });
    if !valid {
        return Err("SEC 요청 연락처는 유효한 이메일 주소여야 합니다.".to_owned());
    }
    Ok(contact.to_owned())
}

fn user_agent(contact: &str) -> String {
    format!("Investa local research app {contact}")
}

fn normalize_ticker(value: &str) -> String {
    value.trim().to_ascii_uppercase().replace('.', "-")
}

impl SecFundamentalsBridge {
    async fn load_tickers(&self, contact: &str) -> Result<HashMap<String, SecTickerEntry>, String> {
        if let Some(entries) = self
            .ticker_cache
            .lock()
            .map_err(|_| "SEC 종목 캐시 잠금을 얻지 못했습니다.".to_owned())?
            .as_ref()
            .filter(|cache| cache.fetched_at.elapsed() < TICKER_CACHE_TTL)
            .map(|cache| cache.entries.clone())
        {
            return Ok(entries);
        }

        let response = self
            .client
            .get(TICKERS_URL)
            .header("User-Agent", user_agent(contact))
            .send()
            .await
            .map_err(|_| "SEC 종목 식별자 목록을 불러오지 못했습니다.".to_owned())?;
        if !response.status().is_success() {
            return Err(format!(
                "SEC 종목 식별자 요청이 실패했습니다. HTTP {}",
                response.status().as_u16()
            ));
        }
        let raw = response
            .json::<HashMap<String, SecTickerEntry>>()
            .await
            .map_err(|_| "SEC 종목 식별자 응답 형식을 확인하지 못했습니다.".to_owned())?;
        let entries = raw
            .into_values()
            .map(|entry| (normalize_ticker(&entry.ticker), entry))
            .collect::<HashMap<_, _>>();
        if entries.is_empty() {
            return Err("SEC 종목 식별자 목록이 비어 있습니다.".to_owned());
        }
        *self
            .ticker_cache
            .lock()
            .map_err(|_| "SEC 종목 캐시 잠금을 얻지 못했습니다.".to_owned())? = Some(TickerCache {
            fetched_at: Instant::now(),
            entries: entries.clone(),
        });
        Ok(entries)
    }

    async fn fetch_company_facts(&self, contact: &str, cik: u64) -> Result<Value, String> {
        let url = format!("{COMPANY_FACTS_BASE_URL}/CIK{cik:010}.json");
        let response = self
            .client
            .get(url)
            .header("User-Agent", user_agent(contact))
            .send()
            .await
            .map_err(|_| "SEC Company Facts를 불러오지 못했습니다.".to_owned())?;
        if !response.status().is_success() {
            return Err(format!(
                "SEC Company Facts 요청이 실패했습니다. HTTP {}",
                response.status().as_u16()
            ));
        }
        response
            .json::<Value>()
            .await
            .map_err(|_| "SEC Company Facts 응답 형식을 확인하지 못했습니다.".to_owned())
    }

    async fn fetch_submissions(&self, contact: &str, cik: u64) -> Result<Value, String> {
        let url = format!("{SUBMISSIONS_BASE_URL}/CIK{cik:010}.json");
        let response = self
            .client
            .get(url)
            .header("User-Agent", user_agent(contact))
            .send()
            .await
            .map_err(|_| "SEC Submissions를 불러오지 못했습니다.".to_owned())?;
        if !response.status().is_success() {
            return Err(format!(
                "SEC Submissions 요청이 실패했습니다. HTTP {}",
                response.status().as_u16()
            ));
        }
        response
            .json::<Value>()
            .await
            .map_err(|_| "SEC Submissions 응답 형식을 확인하지 못했습니다.".to_owned())
    }
}

fn unix_ms_to_date(timestamp_ms: u64) -> String {
    let days = (timestamp_ms / 86_400_000) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn fact_value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn allowed_form(form: &str) -> bool {
    matches!(
        form,
        "10-K"
            | "10-K/A"
            | "10-Q"
            | "10-Q/A"
            | "20-F"
            | "20-F/A"
            | "40-F"
            | "40-F/A"
            | "6-K"
            | "6-K/A"
    )
}

fn latest_metric(
    company_facts: &Value,
    key: &str,
    label: &str,
    tags: &[&str],
    preferred_units: &[&str],
    as_of_date: &str,
) -> Option<SecFundamentalMetric> {
    for tag in tags {
        let Some(fact) = company_facts.pointer(&format!("/facts/us-gaap/{tag}")) else {
            continue;
        };
        let Some(units) = fact.get("units").and_then(Value::as_object) else {
            continue;
        };
        for unit in preferred_units {
            let Some(observations) = units.get(*unit).and_then(Value::as_array) else {
                continue;
            };
            let latest = observations
                .iter()
                .filter_map(|item| {
                    let filed = item.get("filed")?.as_str()?;
                    let end = item.get("end")?.as_str()?;
                    let form = item.get("form")?.as_str()?;
                    (filed < as_of_date && end <= as_of_date && allowed_form(form)).then_some(item)
                })
                .max_by_key(|item| {
                    (
                        item.get("filed")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        item.get("end").and_then(Value::as_str).unwrap_or_default(),
                        item.get("accn").and_then(Value::as_str).unwrap_or_default(),
                    )
                });
            if let Some(item) = latest {
                return Some(SecFundamentalMetric {
                    key: key.to_owned(),
                    label: label.to_owned(),
                    value: fact_value_as_string(item.get("val")?)?,
                    unit: (*unit).to_owned(),
                    period_start: item.get("start").and_then(Value::as_str).map(str::to_owned),
                    period_end: item.get("end")?.as_str()?.to_owned(),
                    filed_at: item.get("filed")?.as_str()?.to_owned(),
                    form: item.get("form")?.as_str()?.to_owned(),
                    accession_no: item.get("accn")?.as_str()?.to_owned(),
                    fiscal_year: item.get("fy").and_then(Value::as_i64),
                    fiscal_period: item.get("fp").and_then(Value::as_str).map(str::to_owned),
                    frame: item.get("frame").and_then(Value::as_str).map(str::to_owned),
                });
            }
        }
    }
    None
}

fn parse_snapshot(
    company_facts: &Value,
    ticker: &str,
    cik: u64,
    as_of_ms: u64,
) -> SecFundamentalSnapshot {
    let as_of_date = unix_ms_to_date(as_of_ms);
    let specifications: [(&str, &str, &[&str], &[&str]); 7] = [
        (
            "revenue",
            "매출",
            &[
                "RevenueFromContractWithCustomerExcludingAssessedTax",
                "Revenues",
                "SalesRevenueNet",
            ],
            &["USD"],
        ),
        (
            "operatingIncome",
            "영업이익",
            &["OperatingIncomeLoss"],
            &["USD"],
        ),
        ("netIncome", "순이익", &["NetIncomeLoss"], &["USD"]),
        ("assets", "자산", &["Assets"], &["USD"]),
        (
            "equity",
            "자본",
            &[
                "StockholdersEquity",
                "StockholdersEquityIncludingPortionAttributableToNoncontrollingInterest",
            ],
            &["USD"],
        ),
        (
            "cash",
            "현금및현금성자산",
            &["CashAndCashEquivalentsAtCarryingValue"],
            &["USD"],
        ),
        (
            "dilutedEps",
            "희석 EPS",
            &["EarningsPerShareDiluted"],
            &["USD/shares"],
        ),
    ];
    let mut metrics = Vec::new();
    let mut missing_metrics = Vec::new();
    for (key, label, tags, units) in specifications {
        match latest_metric(company_facts, key, label, tags, units, &as_of_date) {
            Some(metric) => metrics.push(metric),
            None => missing_metrics.push(label.to_owned()),
        }
    }
    SecFundamentalSnapshot {
        provider: "SEC_COMPANY_FACTS",
        cik: format!("{cik:010}"),
        ticker: normalize_ticker(ticker),
        entity_name: company_facts
            .get("entityName")
            .and_then(Value::as_str)
            .unwrap_or("확인 불가")
            .to_owned(),
        as_of_date,
        metrics,
        missing_metrics,
    }
}

fn relevant_filing_form(form: &str) -> bool {
    matches!(
        form,
        "8-K"
            | "8-K/A"
            | "10-K"
            | "10-K/A"
            | "10-Q"
            | "10-Q/A"
            | "20-F"
            | "20-F/A"
            | "40-F"
            | "40-F/A"
            | "6-K"
            | "6-K/A"
            | "DEF 14A"
            | "DEFA14A"
            | "SC 13D"
            | "SC 13D/A"
            | "SC 13G"
            | "SC 13G/A"
            | "4"
            | "4/A"
    )
}

fn safe_accession(value: &str) -> bool {
    value.len() <= 32
        && !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
}

fn optional_array_string(value: &Value, pointer: &str, index: usize) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .and_then(|values| values.get(index))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

fn parse_filings(
    submissions: &Value,
    ticker: &str,
    cik: u64,
    as_of_ms: u64,
) -> Result<SecFilingSnapshot, String> {
    let accessions = submissions
        .pointer("/filings/recent/accessionNumber")
        .and_then(Value::as_array)
        .ok_or_else(|| "SEC 공시 응답에 accessionNumber 목록이 없습니다.".to_owned())?;
    let forms = submissions
        .pointer("/filings/recent/form")
        .and_then(Value::as_array)
        .ok_or_else(|| "SEC 공시 응답에 form 목록이 없습니다.".to_owned())?;
    let filing_dates = submissions
        .pointer("/filings/recent/filingDate")
        .and_then(Value::as_array)
        .ok_or_else(|| "SEC 공시 응답에 filingDate 목록이 없습니다.".to_owned())?;
    if accessions.len() != forms.len() || accessions.len() != filing_dates.len() {
        return Err("SEC 공시 응답의 필수 배열 길이가 서로 다릅니다.".to_owned());
    }

    let as_of_date = unix_ms_to_date(as_of_ms);
    let cik_path = cik.to_string();
    let mut filings = Vec::new();
    for index in 0..accessions.len() {
        let Some(accession_no) = accessions[index].as_str() else {
            continue;
        };
        let Some(form) = forms[index].as_str() else {
            continue;
        };
        let Some(filed_at) = filing_dates[index].as_str() else {
            continue;
        };
        if filed_at >= as_of_date.as_str()
            || !relevant_filing_form(form)
            || !safe_accession(accession_no)
        {
            continue;
        }
        let accession_path = accession_no.replace('-', "");
        filings.push(SecFilingRecord {
            accession_no: accession_no.to_owned(),
            form: form.to_owned(),
            filed_at: filed_at.to_owned(),
            report_date: optional_array_string(
                submissions,
                "/filings/recent/reportDate",
                index,
            ),
            primary_document: optional_array_string(
                submissions,
                "/filings/recent/primaryDocument",
                index,
            ),
            description: optional_array_string(
                submissions,
                "/filings/recent/primaryDocDescription",
                index,
            ),
            items: optional_array_string(submissions, "/filings/recent/items", index)
                .map(|items| {
                    items
                        .split(',')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .take(20)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            filing_index_url: format!(
                "https://www.sec.gov/Archives/edgar/data/{cik_path}/{accession_path}/{accession_no}-index.html"
            ),
        });
        if filings.len() == 20 {
            break;
        }
    }
    Ok(SecFilingSnapshot {
        provider: "SEC_SUBMISSIONS",
        cik: format!("{cik:010}"),
        ticker: normalize_ticker(ticker),
        entity_name: submissions
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("확인 불가")
            .to_owned(),
        as_of_date,
        filings,
    })
}

pub async fn snapshot_for_ticker(
    bridge: &SecFundamentalsBridge,
    ticker: &str,
    as_of_ms: u64,
) -> Result<Option<SecFundamentalSnapshot>, String> {
    let Some(contact) = load_contact()? else {
        return Ok(None);
    };
    let entries = bridge.load_tickers(&contact).await?;
    let normalized = normalize_ticker(ticker);
    let entry = entries
        .get(&normalized)
        .ok_or_else(|| "SEC에서 해당 미국주식의 CIK를 찾지 못했습니다.".to_owned())?;
    let facts = bridge.fetch_company_facts(&contact, entry.cik_str).await?;
    Ok(Some(parse_snapshot(
        &facts,
        &entry.ticker,
        entry.cik_str,
        as_of_ms,
    )))
}

pub async fn filings_for_ticker(
    bridge: &SecFundamentalsBridge,
    ticker: &str,
    as_of_ms: u64,
) -> Result<Option<SecFilingSnapshot>, String> {
    let Some(contact) = load_contact()? else {
        return Ok(None);
    };
    let entries = bridge.load_tickers(&contact).await?;
    let normalized = normalize_ticker(ticker);
    let entry = entries
        .get(&normalized)
        .ok_or_else(|| "SEC에서 해당 미국주식의 CIK를 찾지 못했습니다.".to_owned())?;
    let submissions = bridge.fetch_submissions(&contact, entry.cik_str).await?;
    Ok(Some(parse_filings(
        &submissions,
        &entry.ticker,
        entry.cik_str,
        as_of_ms,
    )?))
}

#[tauri::command]
pub fn sec_connection_status() -> Result<SecConnectionStatus, String> {
    Ok(match load_contact()? {
        Some(_) => SecConnectionStatus {
            configured: true,
            connected: false,
            message: "SEC 연락처가 저장되어 있습니다. 다음 분석 요청에서 연결을 확인합니다."
                .to_owned(),
        },
        None => SecConnectionStatus {
            configured: false,
            connected: false,
            message: "미장 공식 재무를 사용하려면 SEC 요청 연락처를 등록해 주세요.".to_owned(),
        },
    })
}

#[tauri::command]
pub async fn sec_save_contact(
    request: SecContactRequest,
    bridge: State<'_, SecFundamentalsBridge>,
) -> Result<SecConnectionStatus, String> {
    let contact = validate_contact(&request.contact_email)?;
    bridge.load_tickers(&contact).await?;
    contact_entry()?
        .set_password(&contact)
        .map_err(|_| "SEC 연락처를 Windows 자격 증명 관리자에 저장하지 못했습니다.".to_owned())?;
    Ok(SecConnectionStatus {
        configured: true,
        connected: true,
        message:
            "SEC 종목 식별자 연결을 확인했습니다. 미장 분석에 공식 Company Facts를 사용합니다."
                .to_owned(),
    })
}

#[tauri::command]
pub fn sec_delete_contact() -> Result<SecConnectionStatus, String> {
    match contact_entry()?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(SecConnectionStatus {
            configured: false,
            connected: false,
            message: "SEC 연락처를 삭제했습니다.".to_owned(),
        }),
        Err(_) => Err("SEC 연락처를 Windows 자격 증명 관리자에서 삭제하지 못했습니다.".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_epoch_to_sec_filing_date() {
        assert_eq!(unix_ms_to_date(0), "1970-01-01");
        assert_eq!(unix_ms_to_date(1_774_137_600_000), "2026-03-22");
    }

    #[test]
    fn point_in_time_parser_excludes_future_filings() {
        let facts = json!({
            "entityName": "Example Corp",
            "facts": {"us-gaap": {"Assets": {"units": {"USD": [
                {"end":"2025-12-31","val":100,"accn":"old","form":"10-K","filed":"2026-02-10"},
                {"end":"2026-03-31","val":999,"accn":"future","form":"10-Q","filed":"2026-05-10"}
            ]}}}}
        });
        let snapshot = parse_snapshot(&facts, "TEST", 1, 1_772_323_200_000);
        let assets = snapshot
            .metrics
            .iter()
            .find(|metric| metric.key == "assets")
            .expect("assets");
        assert_eq!(assets.value, "100");
        assert_eq!(assets.accession_no, "old");
    }

    #[test]
    fn parser_rejects_non_periodic_forms_and_reports_missing_metrics() {
        let facts = json!({
            "entityName": "Example Corp",
            "facts": {"us-gaap": {"NetIncomeLoss": {"units": {"USD": [
                {"start":"2025-01-01","end":"2025-12-31","val":50,"accn":"eight-k","form":"8-K","filed":"2026-01-10"}
            ]}}}}
        });
        let snapshot = parse_snapshot(&facts, "TEST", 1, 1_774_137_600_000);
        assert!(snapshot.metrics.is_empty());
        assert!(snapshot.missing_metrics.contains(&"순이익".to_owned()));
    }

    #[test]
    fn parser_excludes_same_date_filings_without_a_reliable_filing_time() {
        let facts = json!({
            "entityName": "Example Corp",
            "facts": {"us-gaap": {"Assets": {"units": {"USD": [
                {"end":"2025-12-31","val":100,"accn":"known-before","form":"10-K","filed":"2026-03-21"},
                {"end":"2025-12-31","val":200,"accn":"same-date","form":"10-K/A","filed":"2026-03-22"}
            ]}}}}
        });
        let snapshot = parse_snapshot(&facts, "TEST", 1, 1_774_137_600_000);
        let assets = snapshot
            .metrics
            .iter()
            .find(|metric| metric.key == "assets")
            .expect("assets");
        assert_eq!(assets.accession_no, "known-before");
    }

    #[test]
    fn validates_contact_without_echoing_it_in_status() {
        assert!(validate_contact("owner@example.com").is_ok());
        assert!(validate_contact("invalid").is_err());
        assert!(!user_agent("owner@example.com").contains('\n'));
    }

    #[test]
    fn filing_parser_excludes_same_date_future_and_irrelevant_forms() {
        let submissions = json!({
            "name":"Example Corp",
            "filings":{"recent":{
                "accessionNumber":["0000000001-26-000001","0000000001-26-000002","0000000001-26-000003","0000000001-26-000004"],
                "form":["8-K","10-Q","S-1","4"],
                "filingDate":["2026-03-21","2026-03-22","2026-03-20","2026-03-23"],
                "reportDate":["2026-03-20","2026-03-21","2026-03-19","2026-03-22"],
                "primaryDocument":["a.htm","b.htm","c.htm","d.htm"],
                "primaryDocDescription":["Material event","Quarterly report","Registration","Ownership"],
                "items":["1.01,2.02","","","4"]
            }}
        });
        let snapshot = parse_filings(&submissions, "TEST", 1, 1_774_137_600_000).expect("filings");
        assert_eq!(snapshot.filings.len(), 1);
        assert_eq!(snapshot.filings[0].form, "8-K");
        assert_eq!(snapshot.filings[0].items, vec!["1.01", "2.02"]);
    }

    #[test]
    fn filing_parser_rejects_misaligned_required_arrays() {
        let submissions = json!({"filings":{"recent":{
            "accessionNumber":["0000000001-26-000001"],
            "form":["8-K","10-Q"],
            "filingDate":["2026-03-21"]
        }}});
        assert!(parse_filings(&submissions, "TEST", 1, 1_774_137_600_000).is_err());
    }
}
