use std::process::Command;

const KIS_PAPER_ACCOUNT_URL: &str =
    "https://securities.koreainvestment.com/main/research/virtual/_static/TF07da010000.jsp";
const KIS_API_PORTAL_URL: &str = "https://apiportal.koreainvestment.com/";

fn official_url(target: &str) -> Result<&'static str, String> {
    match target {
        "kis-paper-account" => Ok(KIS_PAPER_ACCOUNT_URL),
        "kis-api-portal" => Ok(KIS_API_PORTAL_URL),
        _ => Err("허용되지 않은 외부 페이지입니다.".to_owned()),
    }
}

#[tauri::command]
pub fn open_official_external_page(target: String) -> Result<(), String> {
    let url = official_url(target.trim())?;
    Command::new("explorer.exe")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|_| "기본 브라우저를 열지 못했습니다. 잠시 후 다시 시도해 주세요.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_allows_predefined_kis_pages() {
        assert_eq!(official_url("kis-api-portal"), Ok(KIS_API_PORTAL_URL));
        assert!(official_url("https://example.com").is_err());
    }
}
